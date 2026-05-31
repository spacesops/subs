//! Application state for the subsd server.

use std::path::PathBuf;
use std::sync::Arc;
use subs_core::Operator;

use crate::config::ConfigStore;

#[cfg(feature = "test-rig")]
use crate::testrig::TestRigHandle;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub operator: Arc<Operator>,
    /// Global configuration store
    pub config: Arc<ConfigStore>,
    /// Spaced RPC URL for the console
    pub spaced_rpc_url: Option<String>,
    /// Spaced RPC username for proxied calls
    pub spaced_rpc_user: Option<String>,
    /// Spaced RPC password for proxied calls
    pub spaced_rpc_password: Option<String>,
    /// Spaced RPC cookie file for proxied calls (used when user/password not set)
    pub spaced_rpc_cookie: Option<PathBuf>,
    /// HTTP Basic auth credentials (user, password); auth is enforced when Some
    pub basic_auth: Option<(String, String)>,
    /// Bitcoin RPC URL (only available in test-rig mode)
    pub bitcoin_rpc_url: Option<String>,
    /// Certrelay URL (only available in test-rig mode)
    pub certrelay_url: Option<String>,
    /// Test rig handle for mining (only in test-rig mode)
    #[cfg(feature = "test-rig")]
    pub test_rig: Option<Arc<TestRigHandle>>,
}

impl AppState {
    #[cfg(not(feature = "test-rig"))]
    #[allow(clippy::too_many_arguments)]
    pub fn with_rpc_urls(
        operator: Operator,
        config: ConfigStore,
        spaced_rpc_url: Option<String>,
        spaced_rpc_user: Option<String>,
        spaced_rpc_password: Option<String>,
        spaced_rpc_cookie: Option<PathBuf>,
        basic_auth: Option<(String, String)>,
        _bitcoin_rpc_url: Option<String>,
    ) -> Self {
        Self {
            operator: Arc::new(operator),
            config: Arc::new(config),
            spaced_rpc_url,
            spaced_rpc_user,
            spaced_rpc_password,
            spaced_rpc_cookie,
            basic_auth,
            bitcoin_rpc_url: None,
            certrelay_url: None,
        }
    }

    #[cfg(feature = "test-rig")]
    #[allow(clippy::too_many_arguments)]
    pub fn with_rpc_urls(
        operator: Operator,
        config: ConfigStore,
        spaced_rpc_url: Option<String>,
        spaced_rpc_user: Option<String>,
        spaced_rpc_password: Option<String>,
        spaced_rpc_cookie: Option<PathBuf>,
        basic_auth: Option<(String, String)>,
        bitcoin_rpc_url: Option<String>,
    ) -> Self {
        Self {
            operator: Arc::new(operator),
            config: Arc::new(config),
            spaced_rpc_url,
            spaced_rpc_user,
            spaced_rpc_password,
            spaced_rpc_cookie,
            basic_auth,
            bitcoin_rpc_url,
            certrelay_url: None,
            test_rig: None,
        }
    }

    #[cfg(feature = "test-rig")]
    #[allow(clippy::too_many_arguments)]
    pub fn with_test_rig(
        operator: Operator,
        config: ConfigStore,
        spaced_rpc_url: Option<String>,
        spaced_rpc_user: Option<String>,
        spaced_rpc_password: Option<String>,
        spaced_rpc_cookie: Option<PathBuf>,
        basic_auth: Option<(String, String)>,
        bitcoin_rpc_url: Option<String>,
        certrelay_url: Option<String>,
        test_rig: Arc<TestRigHandle>,
    ) -> Self {
        Self {
            operator: Arc::new(operator),
            config: Arc::new(config),
            spaced_rpc_url,
            spaced_rpc_user,
            spaced_rpc_password,
            spaced_rpc_cookie,
            basic_auth,
            bitcoin_rpc_url,
            certrelay_url,
            test_rig: Some(test_rig),
        }
    }
}
