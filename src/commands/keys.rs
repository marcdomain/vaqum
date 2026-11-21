//! `keygen`, `pubkey`, `sign`, `verify`: personal keys and signatures.

use anyhow::{Result, bail};

use crate::cli::{KeygenArgs, PubkeyArgs, SignArgs, VerifyArgs};
use crate::keys::{Identity, PublicKey, default_identity_path};
use crate::sign;

pub fn keygen(args: KeygenArgs) -> Result<()> {
    let path = match args.output {
        Some(p) => p,
        None => default_identity_path()?,
    };
    let identity = Identity::generate()?;
    identity.save(&path)?;
    println!("✔ Created your key: {}", path.display());
    println!("  Keep this file secret, and back it up:");
    println!("  without it, archives sent to you can't be opened.");
    println!();
    println!("  Your public key (share it freely):");
    println!("  {}", identity.public().encode());
    println!();
    println!("  Others encrypt for you with:     vaqum compress <files> --to <your public key>");
    println!("  and check your signatures with:  vaqum verify <archive> --from <your public key>");
    Ok(())
}

pub fn pubkey(args: PubkeyArgs) -> Result<()> {
    let identity = Identity::load_or_default(args.identity.as_deref())?;
    println!("{}", identity.public().encode());
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
