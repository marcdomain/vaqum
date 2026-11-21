//! Leak check: spots credentials about to be packed into an archive
//! (`.env` files, private keys, cloud/API tokens), so they aren't shared by
//! accident. Matches are reported by file and kind, never by value.
//!
//! This is a best-effort safety net, not a guarantee: it knows the common
//! token formats and credential file names, not every secret there is.

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;

use regex::bytes::Regex;

use crate::risk::Warning;

/// Only the start of each file is scanned, so huge files stay fast.
const SCAN_BYTES: u64 = 1024 * 1024;

/// File names that usually hold credentials. Matched against the last
/// path component, case-insensitively.
const SECRET_FILES: &[(&str, &str)] = &[
    (".env", "an environment file (often API keys and passwords)"),
    ("id_rsa", "an SSH private key"),
    ("id_dsa", "an SSH private key"),
    ("id_ecdsa", "an SSH private key"),
    ("id_ed25519", "an SSH private key"),
    (".netrc", "a .netrc file (saved logins)"),
    (".pgpass", "a PostgreSQL password file"),
    (".git-credentials", "saved Git credentials"),
    (".npmrc", "an npm config file (may hold a publish token)"),
    (".pypirc", "a PyPI config file (may hold an upload token)"),
    ("credentials", "a credentials file (e.g. AWS)"),
    ("credentials.json", "a credentials file"),
    ("service-account.json", "a cloud service-account key"),
    (
        "terraform.tfstate",
        "Terraform state (often contains secrets)",
    ),
    ("kubeconfig", "a Kubernetes config (cluster credentials)"),
    ("wallet.dat", "a cryptocurrency wallet"),
];

/// Extensions of files that usually hold keys or certificates with keys.
const SECRET_EXTENSIONS: &[(&str, &str)] = &[
    ("pem", "a PEM key/certificate file"),
    ("key", "a private key file"),
    ("p12", "a PKCS#12 key bundle"),
    ("pfx", "a PKCS#12 key bundle"),
    ("jks", "a Java keystore"),
    ("keystore", "a keystore"),
    ("kdbx", "a KeePass password database"),
    ("ovpn", "a VPN profile (may embed keys)"),
];

struct Pattern {
    what: &'static str,
    re: Regex,
}

fn patterns() -> &'static [Pattern] {
    static PATTERNS: OnceLock<Vec<Pattern>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            ("a private key", r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |ENCRYPTED |PGP )?PRIVATE KEY(?: BLOCK)?-----"),
            ("an AWS access key", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
            ("a GitHub token", r"\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{60,})\b"),
            ("a GitLab token", r"\bglpat-[A-Za-z0-9_\-]{20,}\b"),
            ("a Slack token", r"\bxox[abposr]-[A-Za-z0-9-]{10,}\b"),
            ("a Stripe live key", r"\b(?:sk|rk)_live_[A-Za-z0-9]{20,}\b"),
            ("an Anthropic API key", r"\bsk-ant-[A-Za-z0-9_\-]{20,}\b"),
            ("an OpenAI API key", r"\bsk-(?:proj-|svcacct-)?[A-Za-z0-9_\-]{32,}\b"),
            ("a Google API key", r"\bAIza[0-9A-Za-z_\-]{35}\b"),
            ("an npm token", r"\bnpm_[A-Za-z0-9]{36}\b"),
            ("a PyPI token", r"\bpypi-AgEIcHlwaS5vcmc[A-Za-z0-9_\-]{50,}\b"),
            ("a SendGrid key", r"\bSG\.[A-Za-z0-9_\-]{22}\.[A-Za-z0-9_\-]{43}\b"),
            ("a Twilio key", r"\bSK[0-9a-f]{32}\b"),
        ]
        .into_iter()
        .map(|(what, re)| Pattern {
            what,
            re: Regex::new(re).expect("secret patterns are valid"),
        })
        .collect()
    })
}

/// What credentials `rel` (shown as-is in warnings) seems to hold, judging
/// by its name and the start of its contents at `path`.
pub fn scan_file(path: &Path, rel: &str) -> Vec<Warning> {
    let mut found = Vec::new();
    let name = rel.rsplit('/').next().unwrap_or(rel).to_ascii_lowercase();

    let by_name = SECRET_FILES
        .iter()
        .find(|(n, _)| name == *n || (*n == ".env" && name.starts_with(".env.")))
        .map(|(_, what)| *what)
        .or_else(|| {
            let ext = name.rsplit_once('.').map(|(_, e)| e)?;
            SECRET_EXTENSIONS
                .iter()
                .find(|(e, _)| *e == ext)
                .map(|(_, what)| *what)
        });
    // `.env.example` and friends are templates, not secrets.
    let is_template = ["example", "sample", "template", "dist", "vqm"]
        .iter()
        .any(|t| name.ends_with(&format!(".{t}")));
    if let Some(what) = by_name
        && !is_template
    {
        found.push(Warning {
            path: rel.to_string(),
            reason: format!("looks like {what}"),
        });
    }

    let mut head = Vec::new();
    if let Ok(f) = File::open(path) {
        let _ = f.take(SCAN_BYTES).read_to_end(&mut head);
    }
    for p in patterns() {
        if p.re.is_match(&head) {
            found.push(Warning {
                path: rel.to_string(),
                reason: format!("contains what looks like {}", p.what),
            });
        }
    }
    found
}

/// Prints the leak-check warnings (to stderr).
pub fn print(warnings: &[Warning], encrypted: bool) {
    if warnings.is_empty() {
        return;
    }
    let files = {
        let mut paths: Vec<&str> = warnings.iter().map(|w| w.path.as_str()).collect();
        paths.dedup();
        paths.len()
    };
    eprintln!();
    eprintln!(
        "⚠  Possible secrets in {files} file{} you just packed:",
        if files == 1 { "" } else { "s" }
    );
    for w in warnings {
        eprintln!("   {} — {}", w.path, w.reason);
    }
    if encrypted {
        eprintln!("   The archive is encrypted, so only people with the key can read these.");
    } else {
        eprintln!("   Not encrypted: anyone you share it with can read them.");
        eprintln!("   Leave them out with --exclude, or encrypt with -e or --to.");
        eprintln!("   (--no-secret-check skips this check.)");
    }
}
