//! `~/.config/vaqum/config.toml` (XDG on Linux/macOS, `%APPDATA%\vaqum` on
//! Windows): persisted defaults and named profiles for `compress`, merged
//! under CLI flags at `CLI > --profile > [compress] > [defaults] > built-in`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

pub const FILE_NAME: &str = "config.toml";

#[derive(Deserialize, Default, Clone)]
pub struct FieldSet {
    pub level: Option<u8>,
    pub max: Option<bool>,
    pub threads: Option<usize>,
    pub dedup: Option<bool>,
    pub encrypt: Option<bool>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

impl FieldSet {
    /// `other`'s fields win where set; unset fields fall back to `self`.
    /// `exclude` is additive rather than overriding, since more skip
    /// patterns can't hurt.
    fn merge(self, other: &FieldSet) -> FieldSet {
        FieldSet {
            level: other.level.or(self.level),
            max: other.max.or(self.max),
            threads: other.threads.or(self.threads),
            dedup: other.dedup.or(self.dedup),
            encrypt: other.encrypt.or(self.encrypt),
            exclude: self
                .exclude
                .into_iter()
                .chain(other.exclude.iter().cloned())
                .collect(),
        }
    }
}

#[derive(Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub defaults: FieldSet,
    #[serde(default)]
    pub compress: FieldSet,
    #[serde(default, rename = "profile")]
    pub profiles: BTreeMap<String, FieldSet>,
}

impl Config {
    /// Load from disk, or all-defaults if there's no config file yet.
    pub fn load() -> Result<Self> {
        let path = path()?;
        if !path.is_file() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
    }

    /// `[compress]` merged over `[defaults]`, then `profile` (if named)
    /// merged over that — everything but the invoking command's own CLI
    /// flags, which always win over this.
    pub fn resolve_compress(&self, profile: Option<&str>) -> Result<FieldSet> {
        let resolved = self.defaults.clone().merge(&self.compress);
        match profile {
            Some(name) => {
                let profile = self.profiles.get(name).with_context(|| {
                    format!("no profile named '{name}' (see `vaqum config show`)")
                })?;
                Ok(resolved.merge(profile))
            }
            None => Ok(resolved),
        }
    }
}

/// `~/.config/vaqum/config.toml` (`%APPDATA%\vaqum\config.toml` on
/// Windows), respecting `$XDG_CONFIG_HOME` when set.
pub fn path() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let base = std::env::var_os("APPDATA").context("%APPDATA% isn't set")?;
        Ok(PathBuf::from(base).join("vaqum").join(FILE_NAME))
    }
    #[cfg(not(windows))]
    {
        let base = match std::env::var_os("XDG_CONFIG_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => {
                let home = std::env::var_os("HOME").context("$HOME isn't set")?;
                PathBuf::from(home).join(".config")
            }
        };
        Ok(base.join("vaqum").join(FILE_NAME))
    }
}

pub const STARTER: &str = r#"# vaqum config — see `vaqum config show` for what's currently resolved.
#
# [defaults] applies to every compress; [compress] overrides it for
# compress specifically; a named [profile.<name>] overrides both when
# selected with `compress --profile <name>`. Command-line flags always win
# over all three.

# [defaults]
# level = 9          # 1-22, zstd scale
# max = false         # use LZMA/xz max-compression mode instead of zstd
# threads = 0         # 0 = all cores
# dedup = false
# exclude = ["*.log"]

# [profile.backup]
# max = true
# encrypt = true

# [profile.quick]
# level = 3
"#;

pub fn ensure_not_present() -> Result<PathBuf> {
    let path = path()?;
    if path.exists() {
        bail!(
            "{} already exists; remove it first if you want to start over",
            path.display()
        );
    }
    Ok(path)
}
