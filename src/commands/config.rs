//! `vaqum config show|path|init` — inspect and scaffold the persisted
//! defaults/profiles config that `compress` reads.

use std::fs;

use anyhow::{Context, Result};

use crate::cli::ConfigAction;
use crate::config::{self, Config, FieldSet};

pub fn run(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Show => show(),
        ConfigAction::Path => path(),
        ConfigAction::Init => init(),
    }
}

fn show() -> Result<()> {
    let path = config::path()?;
    if path.is_file() {
        println!("config file: {}", path.display());
    } else {
        println!(
            "config file: {} (not found — showing built-in defaults)",
            path.display()
        );
    }

    let cfg = Config::load()?;
    let resolved = cfg.resolve_compress(None)?;
    println!("\ncompress defaults (`[defaults]` + `[compress]`, before any --profile):");
    print_field_set(&resolved);

    if cfg.profiles.is_empty() {
        println!("\nprofiles: (none)");
    } else {
        println!("\nprofiles:");
        for (name, fields) in &cfg.profiles {
            println!("  {name}:");
            print_field_set(fields);
        }
    }
    Ok(())
}

fn print_field_set(fields: &FieldSet) {
    if let Some(level) = fields.level {
        println!("    level:    {level}");
    }
    if let Some(max) = fields.max {
        println!("    max:      {max}");
    }
    if let Some(threads) = fields.threads {
        println!("    threads:  {threads}");
    }
    if let Some(dedup) = fields.dedup {
        println!("    dedup:    {dedup}");
    }
    if let Some(encrypt) = fields.encrypt {
        println!("    encrypt:  {encrypt}");
    }
    if !fields.exclude.is_empty() {
        println!("    exclude:  {}", fields.exclude.join(", "));
    }
}

fn path() -> Result<()> {
    println!("{}", config::path()?.display());
    Ok(())
}

fn init() -> Result<()> {
    let path = config::ensure_not_present()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&path, config::STARTER)
        .with_context(|| format!("failed to write {}", path.display()))?;
    println!("✔ Wrote starter config -> {}", path.display());
    Ok(())
}
