//! Environment variable and `.env` file loading.

use std::path::Path;

use anyhow::Result;
use clap::ArgMatches;
use config_origins::{
    self as origins, display_secret, origin_for_env_var, origin_from_clap, DotenvLoad,
};

use crate::config::ConfigStore;

pub use config_origins::load_dotenv;

const PUBLISH_REQUIRE_FINALIZED_ENV: &str = "SUBS_PUBLISH_REQUIRE_FINALIZED";

/// Parse a boolean env var. Unset, empty, and unrecognized values default to `false`.
/// Truthy: `1`, `true`, `yes`, `on` (case-insensitive).
pub fn env_bool_default_false(var: &str) -> bool {
    match std::env::var(var) {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

/// Resolve publish finalization gating.
///
/// Precedence: CLI `--publish-require-finalized` > `SUBS_PUBLISH_REQUIRE_FINALIZED` > `false`.
pub fn resolve_publish_require_finalized(matches: &ArgMatches, cli_flag: bool) -> bool {
    if matches!(
        matches.value_source("publish_require_finalized"),
        Some(clap::parser::ValueSource::CommandLine)
    ) {
        return cli_flag;
    }
    env_bool_default_false(PUBLISH_REQUIRE_FINALIZED_ENV)
}

/// Parsed configuration values used for startup logging.
pub struct StartupValues<'a> {
    pub port: u16,
    pub data_dir: &'a Path,
    pub wallet: Option<&'a str>,
    pub rpc_url: Option<&'a str>,
    pub rpc_user: Option<&'a str>,
    pub rpc_password: Option<&'a str>,
    pub rpc_cookie: Option<&'a Path>,
    pub basic_auth_user: Option<&'a str>,
    pub basic_auth_password: Option<&'a str>,
    pub publish_require_finalized: bool,
    #[cfg(feature = "test-rig")]
    pub test_rig: bool,
    #[cfg(feature = "test-rig")]
    pub test_rig_dir: &'a Path,
}

/// Log effective `subs` configuration and each value's origin.
pub fn log_startup(matches: &ArgMatches, dotenv: &DotenvLoad, cfg: StartupValues<'_>) {
    origins::log_section("subs", dotenv);

    origins::log_entry(
        "port",
        cfg.port,
        origin_from_clap(matches, "port", Some("SUBS_PORT"), dotenv),
    );
    origins::log_entry(
        "data_dir",
        cfg.data_dir.display(),
        origin_from_clap(matches, "data_dir", Some("SUBS_DATA_DIR"), dotenv),
    );

    log_field(
        matches,
        "wallet",
        "SUBS_WALLET",
        cfg.wallet,
        dotenv,
        false,
    );
    log_field(
        matches,
        "rpc_url",
        "SUBS_SPACED_RPC_URL",
        cfg.rpc_url,
        dotenv,
        false,
    );
    log_field(
        matches,
        "rpc_user",
        "SUBS_SPACED_RPC_USER",
        cfg.rpc_user,
        dotenv,
        false,
    );
    log_field(
        matches,
        "rpc_password",
        "SUBS_SPACED_RPC_PASSWORD",
        cfg.rpc_password,
        dotenv,
        true,
    );
    let rpc_cookie = cfg
        .rpc_cookie
        .map(|p| p.display().to_string());
    log_field(
        matches,
        "rpc_cookie",
        "SUBS_SPACED_RPC_COOKIE",
        rpc_cookie.as_deref(),
        dotenv,
        false,
    );

    log_field(
        matches,
        "basic_auth_user",
        "SUBS_BASIC_AUTH_USER",
        cfg.basic_auth_user,
        dotenv,
        false,
    );
    log_field(
        matches,
        "basic_auth_password",
        "SUBS_BASIC_AUTH_PASSWORD",
        cfg.basic_auth_password,
        dotenv,
        true,
    );

    log_env_only("prover_endpoint", "SUBS_PROVER_ENDPOINT", dotenv, false);
    log_env_only("registry_endpoint", "SUBS_REGISTRY_ENDPOINT", dotenv, false);

    let publish_origin = if matches!(
        matches.value_source("publish_require_finalized"),
        Some(clap::parser::ValueSource::CommandLine)
    ) {
        Some(origin_from_clap(
            matches,
            "publish_require_finalized",
            Some(PUBLISH_REQUIRE_FINALIZED_ENV),
            dotenv,
        ))
    } else if std::env::var(PUBLISH_REQUIRE_FINALIZED_ENV).is_ok() {
        origin_for_env_var(PUBLISH_REQUIRE_FINALIZED_ENV, dotenv)
    } else {
        None
    };
    origins::log_entry_optional(
        "publish_require_finalized",
        Some(if cfg.publish_require_finalized {
            "true"
        } else {
            "false"
        }),
        publish_origin,
        false,
    );

    #[cfg(feature = "test-rig")]
    {
        origins::log_entry(
            "test_rig",
            cfg.test_rig,
            origin_from_clap(matches, "test_rig", Some("SUBS_TEST_RIG"), dotenv),
        );
        origins::log_entry(
            "test_rig_dir",
            cfg.test_rig_dir.display(),
            origin_from_clap(matches, "test_rig_dir", Some("SUBS_TEST_RIG_DIR"), dotenv),
        );
    }

    println!(
        "  server_url = http://127.0.0.1:{} (derived from port)",
        cfg.port
    );
}

fn log_field(
    matches: &ArgMatches,
    field_id: &str,
    env_var: &str,
    value: Option<&str>,
    dotenv: &DotenvLoad,
    secret: bool,
) {
    let origin = match matches.value_source(field_id) {
        Some(_) => Some(origin_from_clap(matches, field_id, Some(env_var), dotenv)),
        None if value.is_some() && origin_for_env_var(env_var, dotenv).is_some() => {
            origin_for_env_var(env_var, dotenv)
        }
        None => None,
    };

    if secret {
        let display = display_secret(value);
        if let Some(o) = origin {
            origins::log_entry(field_id, display, o);
        } else {
            println!("  {field_id} = {display}");
        }
    } else {
        origins::log_entry_optional(field_id, value, origin, false);
    }
}

fn log_env_only(name: &str, env_var: &str, dotenv: &DotenvLoad, secret: bool) {
    let value = std::env::var(env_var).ok();
    let origin = origin_for_env_var(env_var, dotenv);
    if secret {
        origins::log_entry_optional(name, value.as_deref().map(|_| "(set)"), origin, true);
    } else {
        origins::log_entry_optional(name, value.as_deref(), origin, false);
    }
}

/// Apply optional runtime settings from the environment into `config.db`.
pub fn apply_runtime_config_from_env(config: &ConfigStore) -> Result<()> {
    if let Ok(url) = std::env::var("SUBS_PROVER_ENDPOINT") {
        let url = url.trim();
        if !url.is_empty() {
            config.set_prover_endpoint(url)?;
        }
    }
    if let Ok(url) = std::env::var("SUBS_REGISTRY_ENDPOINT") {
        let url = url.trim();
        if !url.is_empty() {
            config.set_registry_endpoint(url)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_bool_default_false_unset_is_false() {
        let key = "SUBS_PUBLISH_REQUIRE_FINALIZED_TEST_UNSET";
        std::env::remove_var(key);
        assert!(!env_bool_default_false(key));
    }

    #[test]
    fn env_bool_default_false_truthy_values() {
        let key = "SUBS_PUBLISH_REQUIRE_FINALIZED_TEST_TRUTHY";
        for v in ["1", "true", "TRUE", " yes ", "on"] {
            std::env::set_var(key, v);
            assert!(env_bool_default_false(key), "expected true for {v:?}");
        }
        std::env::remove_var(key);
    }

    #[test]
    fn env_bool_default_false_falsey_values() {
        let key = "SUBS_PUBLISH_REQUIRE_FINALIZED_TEST_FALSEY";
        for v in ["", "0", "false", "no", "off", "maybe"] {
            std::env::set_var(key, v);
            assert!(!env_bool_default_false(key), "expected false for {v:?}");
        }
        std::env::remove_var(key);
    }
}
