//! `keygen`, `pubkey`, `sign`, `verify`: personal keys and signatures.

use anyhow::{Result, bail};

use crate::cli::{KeygenArgs, PassphraseArgs, PubkeyArgs, SignArgs, VerifyArgs};
use crate::keys::{Identity, PublicKey, default_identity_path};
use crate::sign;

pub fn keygen(args: KeygenArgs) -> Result<()> {
    let path = match args.output {
        Some(p) => p,
        None => default_identity_path()?,
    };
    if path.exists() {
        bail!(
            "{} already exists; refusing to overwrite a key (pass a different -o path)",
            path.display()
        );
    }
    let passphrase = if args.no_passphrase {
        None
    } else {
        println!("Choose a passphrase to protect your key (it's never stored anywhere):");
        Some(crate::crypto::prompt_new_passphrase(
            "protecting the new key needs a passphrase",
            "pass --no-passphrase for an unprotected key (e.g. for CI)",
        )?)
    };
    let identity = Identity::generate()?;
    identity.save(&path, passphrase.as_deref())?;
    println!("✔ Created your key: {}", path.display());
    if passphrase.is_some() {
        println!("  Protected by your passphrase. Back the file up: without it (and the");
        println!("  passphrase), archives sent to you can't be opened.");
    } else {
        println!("  Not protected by a passphrase: keep this file secret, and back it up.");
    }
    println!();
    println!("  Your public key (share it freely):");
    println!("  {}", identity.public().encode());
    println!();
    println!("  Others encrypt for you with:     vaqum compress <files> --to <your public key>");
    println!("  and check your signatures with:  vaqum verify <archive> --from <your public key>");
    Ok(())
}

pub fn pubkey(args: PubkeyArgs) -> Result<()> {
    // Never needs the passphrase: the public key is stored readable.
    let public = match args.identity.as_deref() {
        Some(path) => Identity::public_key_of(path)?,
        None => match std::env::var(crate::keys::IDENTITY_ENV) {
            Ok(text) if !text.trim().is_empty() => {
                Identity::load_or_default(None)?.public().clone()
            }
            _ => Identity::public_key_of(&default_identity_path()?)?,
        },
    };
    println!("{}", public.encode());
    Ok(())
}

pub fn passphrase(args: PassphraseArgs) -> Result<()> {
    let path = match args.identity {
        Some(p) => p,
        None => default_identity_path()?,
    };
    let was_protected = Identity::is_protected(&path)?;
    let identity = Identity::load(&path)?;
    if args.remove {
        if !was_protected {
            println!("{} has no passphrase; nothing to do.", path.display());
            return Ok(());
        }
        identity.rewrite(&path, None)?;
        println!("✔ Removed the passphrase from {}", path.display());
        println!("  The key is now unprotected on disk: keep the file secret.");
        return Ok(());
    }
    let new = crate::crypto::prompt_new_passphrase(
        "setting a passphrase needs you to type it",
        "run `vaqum passphrase` in a terminal",
    )?;
    identity.rewrite(&path, Some(&new))?;
    println!(
        "✔ {} the passphrase on {}",
        if was_protected { "Changed" } else { "Set" },
        path.display()
    );
    Ok(())
}

pub fn sign(args: SignArgs) -> Result<()> {
    if !args.path.is_file() {
        bail!("'{}' is not a file", args.path.display());
    }
    let identity = Identity::load_or_default(args.identity.as_deref())?;
    let sig = sign::sign_file(&args.path, &identity)?;
    println!("✔ Signed {} -> {}", args.path.display(), sig.display());
    println!("  Send both files. Recipients check them with:");
    println!(
        "  vaqum verify {} --from {}",
        args.path.display(),
        identity.public().encode()
    );
    Ok(())
}

pub fn verify(args: VerifyArgs) -> Result<()> {
    let signer = PublicKey::load(&args.from)?;
    let comment = sign::verify_file(&args.path, args.signature.as_deref(), &signer)?;
    println!(
        "✔ Good signature: {} was signed by this key and hasn't changed since",
        args.path.display()
    );
    if let Some(ts) = comment
        .split('\t')
        .find_map(|f| f.strip_prefix("timestamp:"))
        .and_then(|t| t.parse::<u64>().ok())
    {
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(ts);
        println!("  signed: {}", crate::util::format_time(when));
    }
    Ok(())
}
