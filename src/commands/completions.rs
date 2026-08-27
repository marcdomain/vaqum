//! `vaqum completions` — print a shell completion script, or write it into
//! that shell's standard user completions directory with `--install`.
//! Never edits an rc file directly; where one still needs a line added to
//! load the script, that line is printed instead.

use std::fs::{self, File};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::CommandFactory;
use clap_complete::{Shell, generate};

use crate::cli::{Cli, CompletionsArgs};

pub fn run(args: CompletionsArgs) -> Result<()> {
    if args.install {
        install(args.shell)
    } else {
        let shell = args.shell.expect("clap enforces this unless --install");
        generate_to(shell, &mut std::io::stdout());
        Ok(())
    }
}

fn generate_to<W: std::io::Write>(shell: Shell, out: &mut W) {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    generate(shell, &mut cmd, name, out);
}

fn install(shell: Option<Shell>) -> Result<()> {
    let shell = match shell {
        Some(s) => s,
        None => detect_shell()?,
    };
    let (path, followup) = target_path(shell)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut file =
        File::create(&path).with_context(|| format!("failed to create {}", path.display()))?;
    generate_to(shell, &mut file);

    println!("✔ Installed {shell} completions -> {}", path.display());
    if let Some(note) = followup {
        println!("{note}");
    }
    Ok(())
}

fn detect_shell() -> Result<Shell> {
    let shell_path = std::env::var("SHELL").context(
        "$SHELL isn't set; pass a shell explicitly, e.g. `vaqum completions --install zsh`",
    )?;
    let name = std::path::Path::new(&shell_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    match name {
        "bash" => Ok(Shell::Bash),
        "zsh" => Ok(Shell::Zsh),
        "fish" => Ok(Shell::Fish),
        "elvish" => Ok(Shell::Elvish),
        other => bail!(
            "couldn't map $SHELL ('{other}') to a supported shell; pass one explicitly, e.g. `vaqum completions --install zsh`"
        ),
    }
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("$HOME isn't set")
}

/// Install path for `shell`, plus a follow-up note when loading it still
/// needs a manual rc-file edit.
fn target_path(shell: Shell) -> Result<(PathBuf, Option<String>)> {
    let home = home_dir()?;
    Ok(match shell {
        Shell::Bash => (
            home.join(".local/share/bash-completion/completions/vaqum"),
            Some(
                "  (picked up automatically if the `bash-completion` package is installed and \
                 sourced from your shell; otherwise add `source <path above>` to ~/.bashrc)"
                    .to_string(),
            ),
        ),
        Shell::Zsh => (
            home.join(".zfunc/_vaqum"),
            Some(
                "  Add to ~/.zshrc (if not already present), then restart your shell:\n    \
                 fpath+=(~/.zfunc)\n    autoload -U compinit && compinit"
                    .to_string(),
            ),
        ),
        Shell::Fish => (home.join(".config/fish/completions/vaqum.fish"), None),
        Shell::PowerShell => (
            home.join(".config/powershell/vaqum_completion.ps1"),
            Some("  Add to your PowerShell profile ($PROFILE): . \"<path above>\"".to_string()),
        ),
        Shell::Elvish => (
            home.join(".config/elvish/vaqum_completion.elv"),
            Some("  Add to your rc.elv: eval (slurp < <path above>)".to_string()),
        ),
        other => bail!("--install doesn't support {other} yet"),
    })
}
