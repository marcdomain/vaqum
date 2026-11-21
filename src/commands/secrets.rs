//! `vaqum secrets ...` and `vaqum run`: encrypted .env files.

use std::io::{IsTerminal, Read};
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::cli::{RunArgs, SecretsAction, SecretsArgs};
use crate::keys::{Identity, PublicKey, default_identity_path};
use crate::secret_store::{SecretsFile, Unlocked, parse_dotenv, quote_dotenv};

fn unlock(file: &Path, identity: Option<&Path>) -> Result<Unlocked> {
    let secrets = SecretsFile::load(file)?;
    let identity = Identity::load_or_default(identity)?;
    secrets.unlock(&identity)
}

/// Your own public key, without unlocking your identity.
fn own_public_key(identity: Option<&Path>) -> Result<PublicKey> {
    match identity {
        Some(path) => Identity::public_key_of(path),
        None => match std::env::var(crate::keys::IDENTITY_ENV) {
            Ok(text) if !text.trim().is_empty() => {
                Ok(Identity::load_or_default(None)?.public().clone())
            }
            _ => Identity::public_key_of(&default_identity_path()?)
                .context("you need a key first: run `vaqum keygen`"),
        },
    }
}

/// A secret's value: typed at a hidden prompt, or read from stdin when
/// piped (`echo "$TOKEN" | vaqum secrets set TOKEN`).
fn read_value(name: &str) -> Result<String> {
    if std::io::stdin().is_terminal() {
        return crate::crypto::prompt_secret(&format!("Value for {name}: "), "setting a secret");
    }
    let mut value = String::new();
    std::io::stdin()
        .read_to_string(&mut value)
        .context("failed to read the value from stdin")?;
    let value = value.strip_suffix('\n').unwrap_or(&value);
    Ok(value.strip_suffix('\r').unwrap_or(value).to_string())
}

pub fn run(args: SecretsArgs) -> Result<()> {
    let file = args.file.as_path();
    let identity = args.identity.as_deref();
    match args.action {
        SecretsAction::Init { members } => {
            if file.exists() {
                bail!("{} already exists", file.display());
            }
            let mut keys = vec![own_public_key(identity)?];
            for m in &members {
                let pk = PublicKey::load(m)?;
                if !keys.iter().any(|k| k.encode() == pk.encode()) {
                    keys.push(pk);
                }
            }
            let mut store = SecretsFile::create(&keys)?;
            store.save(file)?;
            println!(
                "✔ Created {} ({} member{})",
                file.display(),
                keys.len(),
                if keys.len() == 1 { "" } else { "s" }
            );
            println!("  Add secrets with:  vaqum secrets set NAME");
            println!("  It's safe to commit: values are encrypted.");
        }
        SecretsAction::Set { name } => {
            crate::secret_store::check_name(&name)?;
            let mut store = unlock(file, identity)?;
            let existed = store.get(&name)?.is_some();
            let value = read_value(&name)?;
            store.set(&name, &value)?;
            store.save(file)?;
            println!("✔ {} {name}", if existed { "Updated" } else { "Set" });
        }
        SecretsAction::Get { name } => {
            let store = unlock(file, identity)?;
            match store.get(&name)? {
                Some(value) => println!("{value}"),
                None => bail!("no secret named {name} in {}", file.display()),
            }
        }
        SecretsAction::List => {
            let store = SecretsFile::load(file)?;
            for name in store.names() {
                println!("{name}");
            }
        }
        SecretsAction::Rm { name } => {
            let mut store = unlock(file, identity)?;
            if !store.remove(&name) {
                bail!("no secret named {name} in {}", file.display());
            }
            store.save(file)?;
            println!("✔ Removed {name}");
        }
        SecretsAction::Import { path } => {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let pairs = parse_dotenv(&text).with_context(|| format!("in {}", path.display()))?;
            let mut store = unlock(file, identity)?;
            for (name, value) in &pairs {
                store.set(name, value)?;
            }
            store.save(file)?;
            println!(
                "✔ Imported {} secret{} into {}",
                pairs.len(),
                if pairs.len() == 1 { "" } else { "s" },
                file.display()
            );
            println!(
                "  Delete the plain-text {} once you've checked it worked.",
                path.display()
            );
        }
        SecretsAction::Export => {
            let store = unlock(file, identity)?;
            eprintln!("⚠  Printing secrets in plain text.");
            for (name, value) in store.all()? {
                println!("{name}={}", quote_dotenv(&value));
            }
        }
        SecretsAction::Members => {
            for m in &SecretsFile::load(file)?.members {
                println!("{}", m.public);
            }
        }
        SecretsAction::AddMember { pubkey } => {
            let pk = PublicKey::load(&pubkey)?;
            let mut store = unlock(file, identity)?;
            if !store.add_member(&pk)? {
                println!("Already a member.");
                return Ok(());
            }
            store.save(file)?;
            println!("✔ Added a member; they can now open {}", file.display());
        }
        SecretsAction::RemoveMember { pubkey } => {
            let pk = PublicKey::load(&pubkey)?;
            let mut store = unlock(file, identity)?;
            if !store.remove_member(&pk)? {
                bail!("that key isn't a member of {}", file.display());
            }
            store.save(file)?;
            println!(
                "✔ Removed the member and re-encrypted {} with a new key.",
                file.display()
            );
            println!("  ⚠  They may have copies of the old values (and git history keeps old");
            println!("     versions of this file), so rotate any secret they could read.");
        }
    }
    Ok(())
}

/// `vaqum run -- COMMAND`: the secrets become environment variables of the
/// command; plain-text values never touch the disk.
pub fn run_command(args: RunArgs) -> Result<()> {
    let store = unlock(&args.file, args.identity.as_deref())?;
    let secrets = store.all()?;
    let (program, rest) = args.command.split_first().context("no command given")?;
    let mut command = std::process::Command::new(program);
    command.args(rest).envs(secrets);

    #[cfg(unix)]
    {
        // Become the command: it gets signals (Ctrl-C) directly, and no
        // vaqum process lingers holding the secrets.
        use std::os::unix::process::CommandExt;
        let err = command.exec();
        bail!("failed to run {program}: {err}");
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .with_context(|| format!("failed to run {program}"))?;
        std::process::exit(status.code().unwrap_or(1));
    }
}
