//! subsd - HTTP REST API server for subs operations.
//!
//! # Usage
//!
//! Normal mode (connect to existing spaced):
//! ```bash
//! subsd --rpc-url http://localhost:7224 --wallet mywallet --data-dir ./data
//! ```
//!
//! Test rig mode (starts bitcoind + spaced automatically):
//! ```bash
//! subsd --test-rig --test-rig-dir ./testdata
//! ```

mod background;
mod config;
mod env;
mod routes;
mod state;

#[cfg(feature = "test-rig")]
mod testrig;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches, Parser};
use subs_core::Operator;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::ConfigStore;
use crate::state::AppState;

#[derive(Parser)]
#[command(
    name = "subsd",
    about = "HTTP REST API server for subs operations",
    version
)]
struct Cli {
    /// Server port
    #[arg(short, long, env = "SUBS_PORT", default_value = "7777")]
    port: u16,

    /// Data directory for spaces
    #[arg(short, long, env = "SUBS_DATA_DIR", default_value = "./data")]
    data_dir: PathBuf,

    /// Wallet name for signing operations (not required with --test-rig)
    #[arg(short, long, env = "SUBS_WALLET", required_unless_present = "test_rig")]
    wallet: Option<String>,

    /// Spaces RPC URL (not required with --test-rig)
    #[arg(short, long, env = "SUBS_SPACED_RPC_URL", required_unless_present = "test_rig")]
    rpc_url: Option<String>,

    /// RPC username (optional)
    #[arg(long, env = "SUBS_SPACED_RPC_USER")]
    rpc_user: Option<String>,

    /// RPC password (optional)
    #[arg(long, env = "SUBS_SPACED_RPC_PASSWORD")]
    rpc_password: Option<String>,

    /// RPC cookie file path (optional)
    #[arg(long, env = "SUBS_SPACED_RPC_COOKIE")]
    rpc_cookie: Option<PathBuf>,

    /// Enable test rig mode (starts bitcoind + spaced automatically)
    #[cfg(feature = "test-rig")]
    #[arg(long, env = "SUBS_TEST_RIG")]
    test_rig: bool,

    /// Directory for test rig data (persistent across restarts)
    #[cfg(feature = "test-rig")]
    #[arg(long, env = "SUBS_TEST_RIG_DIR", default_value = "./testrig-data")]
    test_rig_dir: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let dotenv = env::load_dotenv("SUBS_ENV_FILE");

    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "subs=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let matches = Cli::command().get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    env::log_startup(
        &matches,
        &dotenv,
        env::StartupValues {
            port: cli.port,
            data_dir: &cli.data_dir,
            wallet: cli.wallet.as_deref(),
            rpc_url: cli.rpc_url.as_deref(),
            rpc_user: cli.rpc_user.as_deref(),
            rpc_password: cli.rpc_password.as_deref(),
            rpc_cookie: cli.rpc_cookie.as_deref(),
            #[cfg(feature = "test-rig")]
            test_rig: cli.test_rig,
            #[cfg(feature = "test-rig")]
            test_rig_dir: &cli.test_rig_dir,
        },
    );

    #[cfg(feature = "test-rig")]
    {
        if cli.test_rig {
            let mut handle = run_with_test_rig(cli).await?;
            // Gracefully stop bitcoind so it flushes blocks to disk
            if let Err(e) = handle.stop().await {
                tracing::warn!("Failed to stop bitcoind cleanly: {}", e);
            }
        } else {
            run_normal(cli).await?;
        }
    }

    #[cfg(not(feature = "test-rig"))]
    run_normal(cli).await?;

    Ok(())
}

async fn run_normal(cli: Cli) -> Result<()> {
    use spaces_client::rpc::RpcClient;

    let rpc_url = cli.rpc_url.as_ref().expect("rpc_url required");
    let wallet = cli.wallet.as_ref().expect("wallet required");

    // Build RPC client
    let rpc = build_rpc_client(rpc_url, cli.rpc_user.as_deref(), cli.rpc_password.as_deref(), cli.rpc_cookie.as_deref())?;

    // Ensure wallet is loaded
    tracing::info!("Loading wallet: {}", wallet);
    if let Err(e) = rpc.wallet_load(wallet).await {
        let err_str = e.to_string();
        // Ignore "already loaded" errors
        if !err_str.contains("already loaded") && !err_str.contains("already exists") {
            anyhow::bail!("Failed to load wallet {}: {}", wallet, e);
        }
        tracing::info!("Wallet {} already loaded", wallet);
    } else {
        tracing::info!("Wallet {} loaded", wallet);
    }

    // Create data directory if needed
    if !cli.data_dir.exists() {
        std::fs::create_dir_all(&cli.data_dir)?;
    }

    // Create config store
    let config_path = cli.data_dir.join("config.db");
    let config = ConfigStore::open(&config_path)?;
    env::apply_runtime_config_from_env(&config)?;

    // Create operator
    let operator = Operator::new(cli.data_dir, wallet, rpc)
        .with_fabric();

    // Load all existing spaces from disk
    operator.load_all_spaces().await?;

    // Build app state and run server
    run_server(
        operator,
        config,
        cli.port,
        Some(rpc_url.clone()),
        cli.rpc_user.clone(),
        cli.rpc_password.clone(),
        cli.rpc_cookie.clone(),
        None,
    )
    .await
}

#[cfg(feature = "test-rig")]
async fn run_with_test_rig(cli: Cli) -> Result<testrig::TestRigHandle> {
    use std::sync::Arc;
    use crate::testrig::TestRigHandle;

    tracing::info!("Starting test rig...");
    tracing::info!("Test rig data directory: {}", cli.test_rig_dir.display());
    tracing::info!("Operator data directory: {}", cli.data_dir.display());

    // Start test rig
    let handle = TestRigHandle::start(&cli.test_rig_dir).await?;
    let handle = Arc::new(handle);

    tracing::info!("Test rig started!");
    tracing::info!("  Bitcoin RPC: {}", handle.bitcoin_rpc_url());
    tracing::info!("  Spaced RPC: {}", handle.spaced_rpc_url());

    // Start certrelay instance
    let certrelay_port = cli.port + 2;
    let (certrelay_url, _certrelay_shutdown) = handle
        .start_certrelay(&cli.data_dir, certrelay_port)
        .await?;
    tracing::info!("  Certrelay: {}", certrelay_url);

    // Build RPC client for spaced
    let rpc = build_rpc_client(
        handle.spaced_rpc_url(),
        Some("user"),
        Some("pass"),
        None,
    )?;

    // Create data directory if needed
    if !cli.data_dir.exists() {
        std::fs::create_dir_all(&cli.data_dir)?;
    }

    // Create config store
    let config_path = cli.data_dir.join("config.db");
    let config = ConfigStore::open(&config_path)?;
    env::apply_runtime_config_from_env(&config)?;

    // Use default wallet from test rig
    let wallet = "wallet_99";
    tracing::info!("Using wallet: {}", wallet);

    // Create operator with certrelay as fabric seed
    let operator = Operator::new(cli.data_dir.clone(), wallet, rpc)
        .with_fabric_seeds(&[&certrelay_url]);

    // Load all existing spaces from disk
    operator.load_all_spaces().await?;

    // Run server (this blocks until shutdown)
    let spaced_url = handle.spaced_rpc_url().to_string();
    let bitcoin_url = handle.bitcoin_rpc_url().to_string();
    run_server_with_testrig(operator, config, cli.port, spaced_url, bitcoin_url, certrelay_url, handle.clone()).await?;

    // Background tasks (proving loop) hold AppState clones with Arc refs.
    // On shutdown just leak them; the process is exiting anyway.
    match Arc::try_unwrap(handle) {
        Ok(h) => Ok(h),
        Err(_) => {
            tracing::info!("Shutting down with process exit");
            std::process::exit(0);
        }
    }
}

async fn run_server(
    operator: Operator,
    config: ConfigStore,
    port: u16,
    spaced_rpc_url: Option<String>,
    spaced_rpc_user: Option<String>,
    spaced_rpc_password: Option<String>,
    spaced_rpc_cookie: Option<PathBuf>,
    bitcoin_rpc_url: Option<String>,
) -> Result<()> {
    // Build app state
    let state = AppState::with_rpc_urls(
        operator,
        config,
        spaced_rpc_url,
        spaced_rpc_user,
        spaced_rpc_password,
        spaced_rpc_cookie,
        bitcoin_rpc_url,
    );
    run_server_inner(state, port).await
}

#[cfg(feature = "test-rig")]
async fn run_server_with_testrig(
    operator: Operator,
    config: ConfigStore,
    port: u16,
    spaced_rpc_url: String,
    bitcoin_rpc_url: String,
    certrelay_url: String,
    test_rig: std::sync::Arc<testrig::TestRigHandle>,
) -> Result<()> {
    // Build app state with test rig
    let state = AppState::with_test_rig(
        operator,
        config,
        Some(spaced_rpc_url),
        Some("user".to_string()),
        Some("pass".to_string()),
        None,
        Some(bitcoin_rpc_url),
        Some(certrelay_url),
        test_rig,
    );
    run_server_inner(state, port).await
}

async fn run_server_inner(state: AppState, port: u16) -> Result<()> {
    // Start background proving loop
    background::spawn_proving_loop(state.clone());

    // Build router
    let app = routes::router()
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state);

    // Start server
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Starting server on http://{}", addr);
    tracing::info!("Server URL: http://127.0.0.1:{}", port);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn build_rpc_client(
    rpc_url: &str,
    rpc_user: Option<&str>,
    rpc_password: Option<&str>,
    rpc_cookie: Option<&std::path::Path>,
) -> Result<spaces_client::jsonrpsee::http_client::HttpClient> {
    use spaces_client::jsonrpsee::http_client::HttpClientBuilder;

    let mut builder = HttpClientBuilder::default();

    // Set auth if provided
    if let Some(user) = rpc_user {
        let password = rpc_password.unwrap_or("");
        let auth = format!("{}:{}", user, password);
        let encoded = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            auth.as_bytes(),
        );
        builder = builder.set_headers(
            std::iter::once((
                "Authorization".parse().unwrap(),
                format!("Basic {}", encoded).parse().unwrap(),
            ))
            .collect(),
        );
    } else if let Some(cookie_path) = rpc_cookie {
        let cookie = std::fs::read_to_string(cookie_path)?;
        let encoded = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            cookie.trim().as_bytes(),
        );
        builder = builder.set_headers(
            std::iter::once((
                "Authorization".parse().unwrap(),
                format!("Basic {}", encoded).parse().unwrap(),
            ))
            .collect(),
        );
    }

    let client = builder.build(rpc_url)?;
    Ok(client)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received, starting graceful shutdown");
}
