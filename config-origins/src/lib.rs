//! Helpers for loading `.env` files and logging effective configuration with origins.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use clap::parser::ValueSource;
use clap::ArgMatches;

/// Where a configuration value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigOrigin {
    /// Command-line flag or positional argument.
    Param,
    /// Process environment (e.g. `export VAR=...`) before `.env` was applied.
    Environment,
    /// `.env` file (or file pointed to by `*_ENV_FILE`).
    DotEnv,
    /// Built-in default when nothing else was provided.
    Default,
}

impl fmt::Display for ConfigOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Param => write!(f, "param"),
            Self::Environment => write!(f, "environment"),
            Self::DotEnv => write!(f, ".env"),
            Self::Default => write!(f, "default"),
        }
    }
}

/// Result of loading a dotenv file.
#[derive(Debug, Clone)]
pub struct DotenvLoad {
    /// Path loaded, if any.
    pub env_file: Option<PathBuf>,
    /// Variable names whose values were applied from the file (not pre-set in the process env).
    pub keys_from_dotenv: HashSet<String>,
}

/// Snapshot of the process environment before loading dotenv.
type EnvSnapshot = HashMap<String, String>;

/// Load variables from a `.env` file before CLI parsing.
///
/// If `env_file_var` is set in the environment, that path is used; otherwise tries `.env`
/// in the current working directory. Existing process environment variables are not overridden.
pub fn load_dotenv(env_file_var: &str) -> DotenvLoad {
    let before = snapshot_env();
    let (env_file, file_keys) = resolve_env_file(env_file_var);

    if let Some(ref path) = env_file {
        let _ = dotenvy::from_filename(path);
    } else {
        let _ = dotenvy::dotenv();
    }

    let mut keys_from_dotenv = HashSet::new();
    for key in file_keys {
        if std::env::var(&key).is_ok() && !before.contains_key(&key) {
            keys_from_dotenv.insert(key);
        }
    }

    DotenvLoad {
        env_file,
        keys_from_dotenv,
    }
}

fn snapshot_env() -> EnvSnapshot {
    std::env::vars().collect()
}

fn resolve_env_file(env_file_var: &str) -> (Option<PathBuf>, HashSet<String>) {
    if let Ok(path) = std::env::var(env_file_var) {
        if !path.is_empty() {
            let p = PathBuf::from(&path);
            let keys = parse_dotenv_keys(&p).unwrap_or_default();
            return (Some(p), keys);
        }
    }

    let dot_env = PathBuf::from(".env");
    if dot_env.is_file() {
        let keys = parse_dotenv_keys(&dot_env).unwrap_or_default();
        (Some(dot_env), keys)
    } else {
        (None, HashSet::new())
    }
}

/// Parse variable names from a dotenv file (ignores comments and blank lines).
fn parse_dotenv_keys(path: &Path) -> std::io::Result<HashSet<String>> {
    let content = std::fs::read_to_string(path)?;
    let mut keys = HashSet::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        if let Some((key, _)) = line.split_once('=') {
            let key = key.trim();
            if !key.is_empty() {
                keys.insert(key.to_string());
            }
        }
    }
    Ok(keys)
}

/// Origin for a clap argument that may also use an environment variable.
pub fn origin_from_clap(
    matches: &ArgMatches,
    field_id: &str,
    env_var: Option<&str>,
    dotenv: &DotenvLoad,
) -> ConfigOrigin {
    match matches.value_source(field_id) {
        Some(ValueSource::CommandLine) => ConfigOrigin::Param,
        Some(ValueSource::EnvVariable) => {
            env_var
                .and_then(|v| origin_for_env_var(v, dotenv))
                .unwrap_or(ConfigOrigin::Environment)
        }
        Some(ValueSource::DefaultValue) => ConfigOrigin::Default,
        _ => ConfigOrigin::Default,
    }
}

/// Origin for a setting that is only available via environment (not a CLI flag).
pub fn origin_for_env_var(env_var: &str, dotenv: &DotenvLoad) -> Option<ConfigOrigin> {
    if std::env::var(env_var).is_err() {
        return None;
    }
    if dotenv.keys_from_dotenv.contains(env_var) {
        Some(ConfigOrigin::DotEnv)
    } else {
        Some(ConfigOrigin::Environment)
    }
}

/// Print a startup configuration section to stdout.
pub fn log_section(component: &str, dotenv: &DotenvLoad) {
    println!("{component} configuration:");
    if let Some(path) = &dotenv.env_file {
        println!("  (loaded env file: {})", path.display());
    }
}

/// Print one configuration line.
pub fn log_entry(name: &str, value: impl fmt::Display, origin: ConfigOrigin) {
    println!("  {name} = {value} ({origin})");
}

/// Print one optional configuration line.
pub fn log_entry_optional(
    name: &str,
    value: Option<impl fmt::Display>,
    origin: Option<ConfigOrigin>,
    secret: bool,
) {
    match (value, origin) {
        (Some(v), Some(o)) => {
            if secret {
                log_entry(name, "(set)", o);
            } else {
                log_entry(name, v, o);
            }
        }
        _ => println!("  {name} = (not set)"),
    }
}

/// Display value for sensitive settings.
pub fn display_secret(value: Option<&str>) -> String {
    if value.is_some() && !value.unwrap_or("").is_empty() {
        "(set)".to_string()
    } else {
        "(not set)".to_string()
    }
}
