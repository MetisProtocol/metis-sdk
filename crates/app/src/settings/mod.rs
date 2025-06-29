#![allow(missing_docs)]
use anyhow::Context;
use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use serde_with::{DurationSeconds, serde_as};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tendermint_rpc::Url;

#[allow(dead_code, unreachable_pub)]
#[derive(Debug, Deserialize, Clone)]
pub struct GasOpt {
    pub min_gas_premium: u64,
    pub num_blocks_max_prio_fee: u64,
    pub max_fee_hist_size: u64,
}

#[allow(dead_code, unreachable_pub)]
#[derive(Debug, Deserialize, Clone)]
pub struct Address {
    pub host: String,
    pub port: u32,
}

#[allow(dead_code, unreachable_pub)]
impl Address {
    pub fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code, unreachable_pub)]
pub struct AbciSettings {
    pub listen: Address,
    /// Queue size for each ABCI component.
    pub bound: usize,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code, unreachable_pub)]
pub struct DbSettings {
    /// Length of the app state history to keep in the database before pruning; 0 means unlimited.
    ///
    /// This affects how long we can go back in state queries.
    pub state_hist_size: u64,
}

/// Ethereum API facade settings.
#[serde_as]
#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code, unreachable_pub)]
pub struct EthSettings {
    pub listen: Address,
    #[serde_as(as = "DurationSeconds<u64>")]
    pub filter_timeout: Duration,
    pub cache_capacity: usize,
    pub gas: GasOpt,
}

#[allow(dead_code, unreachable_pub)]
#[derive(Debug, Deserialize, Clone)]
pub struct Settings {
    /// Home directory configured on the CLI, to which all paths in settings can be set relative.
    home_dir: PathBuf,
    /// Database files.
    data_dir: PathBuf,
    /// Solidity contracts.
    contracts_dir: PathBuf,
    /// Builtin-actors CAR file.
    builtin_actors_bundle: PathBuf,
    /// Where to reach `CometBFT` for queries or broadcasting transactions.
    tendermint_rpc_url: Url,
    /// Secp256k1 private key used for signing transactions. Leave empty if not validating.
    validator_key: PathBuf,
    pub abci: AbciSettings,
    pub db: DbSettings,
    pub eth: EthSettings,
}

#[macro_export]
macro_rules! home_relative {
    // Using this inside something that has a `.home_dir()` function.
    ($($name:ident),+) => {
        $(
        pub fn $name(&self) -> std::path::PathBuf {
            expand_path(&self.home_dir(), &self.$name)
        }
        )+
    };

    // Using this outside something that requires a `home_dir` parameter to be passed to it.
    ($settings:ty { $($name:ident),+ } ) => {
      impl $settings {
        $(
        pub fn $name(&self, home_dir: &std::path::Path) -> std::path::PathBuf {
            $crate::settings::expand_path(home_dir, &self.$name)
        }
        )+
      }
    };
}

#[allow(dead_code, unreachable_pub)]
impl Settings {
    home_relative!(
        data_dir,
        contracts_dir,
        builtin_actors_bundle,
        validator_key
    );

    /// Load the default configuration from a directory,
    /// then potential overrides specific to the run mode,
    /// then overrides from the local environment.
    pub fn new(config_dir: &Path, home_dir: &Path, run_mode: &str) -> Result<Self, ConfigError> {
        let c = Config::builder()
            .add_source(File::from(config_dir.join("default")))
            // Optional mode specific overrides, checked into git.
            .add_source(File::from(config_dir.join(run_mode)).required(false))
            // Optional local overrides, not checked into git.
            .add_source(File::from(config_dir.join("local")).required(false))
            // Add in settings from the environment (with a prefix of FM)
            // e.g. `FM_DB__DATA_DIR=./foo/bar ./target/app` would set the database location.
            .add_source(
                Environment::with_prefix("fm")
                    .prefix_separator("_")
                    .separator("__"),
            )
            // Set the home directory based on what was passed to the CLI,
            // so everything in the config can be relative to it.
            // The `home_dir` key is not added to `default.toml` so there is no confusion
            // about where it will be coming from.
            .set_override("home_dir", home_dir.to_string_lossy().as_ref())?
            .build()?;

        // Deserialize (and thus freeze) the entire configuration.
        c.try_deserialize()
    }

    /// The configured home directory.
    pub fn home_dir(&self) -> &Path {
        &self.home_dir
    }

    /// Tendermint RPC URL from the environment or the config file.
    pub fn tendermint_rpc_url(&self) -> anyhow::Result<Url> {
        // Prefer the "standard" env var used in the CLI.
        match std::env::var("TENDERMINT_RPC_URL").ok() {
            Some(url) => url.parse::<Url>().context("invalid Tendermint URL"),
            None => Ok(self.tendermint_rpc_url.clone()),
        }
    }
}

/// Expand a path which can either be :
/// * absolute, e.g. "/foo/bar"
/// * relative to the system `$HOME` directory, e.g. "~/foo/bar"
/// * relative to the configured `--home-dir` directory, e.g. "foo/bar"
#[allow(dead_code, unreachable_pub)]
pub fn expand_path(home_dir: &Path, path: &Path) -> PathBuf {
    if path.starts_with("/") {
        PathBuf::from(path)
    } else if path.starts_with("~") {
        expand_tilde(path)
    } else {
        expand_tilde(home_dir.join(path))
    }
}

/// Expand paths that begin with "~" to `$HOME`.
#[allow(dead_code, unreachable_pub)]
pub fn expand_tilde<P: AsRef<Path>>(path: P) -> PathBuf {
    let p = path.as_ref().to_path_buf();
    if !p.starts_with("~") {
        return p;
    }
    if p == Path::new("~") {
        return dirs::home_dir().unwrap_or(p);
    }
    dirs::home_dir()
        .map(|mut h| {
            if h == Path::new("/") {
                // `~/foo` becomes just `/foo` instead of `//foo` if `/` is home.
                p.strip_prefix("~").unwrap().to_path_buf()
            } else {
                h.push(p.strip_prefix("~/").unwrap());
                h
            }
        })
        .unwrap_or(p)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::Settings;
    use super::expand_tilde;

    #[allow(dead_code)]
    fn parse_config(run_mode: &str) -> Settings {
        let current_dir = PathBuf::from(".");
        let default_dir = PathBuf::from("config");
        Settings::new(&default_dir, &current_dir, run_mode).unwrap()
    }

    #[test]
    fn tilde_expands_to_home() {
        let home = std::env::var("HOME").expect("should work on Linux");
        let home_project = PathBuf::from(format!("{}/.project", home));
        assert_eq!(expand_tilde("~/.project"), home_project);
        assert_eq!(expand_tilde("/foo/bar"), PathBuf::from("/foo/bar"));
        assert_eq!(expand_tilde("~foo/bar"), PathBuf::from("~foo/bar"));
    }
}
