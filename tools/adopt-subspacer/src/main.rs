//! Adopt a legacy subspacer space directory into subs `SUBS_DATA_DIR`.
//!
//! Copies the SpaceDB tree, verifies the Merkle root against on-chain state,
//! and seeds `subs.db` with a single genesis commitment plus committed handles.

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use serde::Deserialize;
use spaces_client::jsonrpsee::http_client::HttpClientBuilder;
use spaces_client::rpc::RpcClient;
use spaces_protocol::slabel::SLabel;
use spaces_protocol::sname::{SName, Subname};
use spacedb::db::Database;
use spacedb::{Configuration, Sha256Hasher};
use subs_core::storage::Storage;
use subs_core::{Batch, BatchEntry};

#[derive(Parser)]
#[command(about = "Adopt a legacy subspacer space into subs SUBS_DATA_DIR")]
struct Cli {
    /// Path to the subspacer space directory (e.g. data_subspacer/eurt)
    source: PathBuf,

    /// subs data directory (default: $SUBS_DATA_DIR or ./data)
    #[arg(long, env = "SUBS_DATA_DIR", default_value = "./data")]
    data_dir: PathBuf,

    /// On-chain commit transaction id for the adopted tip (recommended)
    #[arg(long)]
    commit_txid: Option<String>,

    /// Overwrite an existing destination space directory
    #[arg(long)]
    force: bool,

    /// Verify only; do not copy files or write subs.db
    #[arg(long)]
    dry_run: bool,

    /// Skip on-chain root verification via spaced RPC
    #[arg(long)]
    skip_rpc_verify: bool,

    #[arg(long, env = "SUBS_SPACED_RPC_URL", default_value = "http://127.0.0.1:7225")]
    rpc_url: String,

    #[arg(long, env = "SUBS_SPACED_RPC_USER")]
    rpc_user: Option<String>,

    #[arg(long, env = "SUBS_SPACED_RPC_PASSWORD")]
    rpc_password: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChainFile {
    space: String,
    entries: Vec<ChainEntry>,
}

#[derive(Debug, Deserialize)]
struct ChainEntry {
    post_diff_root: String,
}

#[derive(Debug, Deserialize)]
struct HandleReq {
    handle: String,
    script_pubkey: String,
}

#[derive(Debug, Deserialize)]
struct UncommittedFile {
    entries: Vec<UncommittedEntry>,
}

#[derive(Debug, Deserialize)]
struct UncommittedEntry {
    sub_label: String,
}

fn compute_sdb_root(path: &Path) -> Result<String> {
    let config = Configuration::standard();
    let db = Database::<Sha256Hasher>::open_with_config(
        path.to_str()
            .ok_or_else(|| anyhow!("non-UTF-8 path: {}", path.display()))?,
        config,
    )
        .with_context(|| format!("could not open SpaceDB file: {}", path.display()))?;
    let mut snap = db.begin_read().context("could not begin read transaction")?;
    let root = snap.compute_root().context("could not compute Merkle root")?;
    Ok(hex::encode(root))
}

fn parse_space(label: &str) -> Result<SLabel> {
    label
        .parse()
        .map_err(|e| anyhow!("invalid space label '{label}': {e}"))
}

fn load_committed_handles(source: &Path) -> Result<Vec<HandleReq>> {
    let mut excluded = HashSet::new();
    let uncommitted_path = source.join("uncommitted.json");
    if uncommitted_path.exists() {
        let raw = fs::read_to_string(&uncommitted_path)
            .with_context(|| format!("read {}", uncommitted_path.display()))?;
        let uncommitted: UncommittedFile = serde_json::from_str(&raw)
            .with_context(|| format!("parse {}", uncommitted_path.display()))?;
        excluded.extend(uncommitted.entries.into_iter().map(|e| e.sub_label));
    }

    let mut handles = Vec::new();
    for entry in fs::read_dir(source).with_context(|| format!("read {}", source.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if !file_name.ends_with(".req.json") {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("read {}", path.display()))?;
        let req: HandleReq =
            serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
        let sub_label = req
            .handle
            .split('@')
            .next()
            .ok_or_else(|| anyhow!("invalid handle in {}: {}", path.display(), req.handle))?;
        if excluded.contains(sub_label) {
            continue;
        }
        handles.push(req);
    }

    handles.sort_by(|a, b| a.handle.cmp(&b.handle));
    Ok(handles)
}

fn build_zk_batch(space: &SLabel, handles: &[HandleReq]) -> Result<Vec<u8>> {
    let mut batch = Batch::new(space.clone());
    for req in handles {
        let handle: SName = req
            .handle
            .parse()
            .map_err(|e| anyhow!("invalid handle '{}': {e}", req.handle))?;
        let sub_label = handle
            .subspace()
            .ok_or_else(|| anyhow!("handle '{}' has no subspace", req.handle))?
            .clone();
        let script_pubkey = hex::decode(&req.script_pubkey)
            .with_context(|| format!("invalid script_pubkey hex for {}", req.handle))?;
        batch.entries.push(BatchEntry {
            sub_label,
            script_pubkey: script_pubkey.into(),
        });
    }
    Ok(batch.to_zk_input())
}

async fn verify_on_chain(
    rpc_url: &str,
    rpc_user: Option<&str>,
    rpc_password: Option<&str>,
    space: &SLabel,
    expected_root: &str,
) -> Result<()> {
    let mut builder = HttpClientBuilder::default();
    if let Some(user) = rpc_user {
        let password = rpc_password.unwrap_or("");
        let auth = format!("{user}:{password}");
        let encoded = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            auth.as_bytes(),
        );
        builder = builder.set_headers(
            std::iter::once((
                "Authorization".parse().unwrap(),
                format!("Basic {encoded}").parse().unwrap(),
            ))
            .collect(),
        );
    }
    let client = builder
        .build(rpc_url)
        .with_context(|| format!("connect to spaced RPC at {rpc_url}"))?;

    let on_chain = client
        .get_commitment(space.clone().into(), None)
        .await
        .context("getcommitment RPC failed")?
        .ok_or_else(|| anyhow!("no on-chain commitment found for {space}"))?;

    let chain_root = hex::encode(on_chain.state_root);
    if chain_root != expected_root {
        bail!(
            "on-chain root mismatch for {space}: chain={chain_root}, expected={expected_root}"
        );
    }

    if let Some(prev) = on_chain.prev_root {
        eprintln!(
            "note: on-chain tip has prev_root={}; adopting as a single genesis row in subs.db",
            hex::encode(prev)
        );
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let source = cli.source.canonicalize().with_context(|| {
        format!("source directory not found: {}", cli.source.display())
    })?;

    let chain_path = source.join("chain.json");
    let chain_raw = fs::read_to_string(&chain_path)
        .with_context(|| format!("read {}", chain_path.display()))?;
    let chain: ChainFile = serde_json::from_str(&chain_raw)
        .with_context(|| format!("parse {}", chain_path.display()))?;
    let space = parse_space(&chain.space)?;
    let tip_root = chain
        .entries
        .last()
        .ok_or_else(|| anyhow!("chain.json has no entries"))?
        .post_diff_root
        .clone();

    let sdb_name = format!("{space}.sdb");
    let source_sdb = source.join(&sdb_name);
    if !source_sdb.is_file() {
        bail!("missing SpaceDB file: {}", source_sdb.display());
    }

    let local_root = compute_sdb_root(&source_sdb)?;
    if local_root != tip_root {
        bail!(
            "local .sdb root does not match chain.json tip: sdb={local_root}, chain={tip_root}"
        );
    }
    println!("verified local .sdb root: {local_root}");

    if !cli.skip_rpc_verify {
        verify_on_chain(
            &cli.rpc_url,
            cli.rpc_user.as_deref(),
            cli.rpc_password.as_deref(),
            &space,
            &tip_root,
        )
        .await?;
        println!("verified on-chain root via RPC");
    }

    let handles = load_committed_handles(&source)?;
    if handles.is_empty() {
        bail!("no committed handles found in {}", source.display());
    }

    let dest_dir = cli.data_dir.join(space.to_string());
    let dest_sdb = dest_dir.join(&sdb_name);
    let dest_db = dest_dir.join("subs.db");

    if dest_dir.exists() && !cli.force {
        bail!(
            "destination already exists: {} (pass --force to overwrite)",
            dest_dir.display()
        );
    }

    println!("space:      {space}");
    println!("source:     {}", source.display());
    println!("dest:       {}", dest_dir.display());
    println!("handles:    {}", handles.len());
    for req in &handles {
        println!("  - {}", req.handle);
    }

    if cli.dry_run {
        println!("dry run complete; no files written");
        return Ok(());
    }

    if dest_dir.exists() {
        fs::remove_dir_all(&dest_dir)
            .with_context(|| format!("remove {}", dest_dir.display()))?;
    }
    fs::create_dir_all(&dest_dir)
        .with_context(|| format!("create {}", dest_dir.display()))?;
    fs::copy(&source_sdb, &dest_sdb).with_context(|| {
        format!(
            "copy {} -> {}",
            source_sdb.display(),
            dest_sdb.display()
        )
    })?;
    println!("copied {}", dest_sdb.display());

    let zk_batch = build_zk_batch(&space, &handles)?;
    let storage = Storage::open(&dest_db)
        .await
        .with_context(|| format!("open {}", dest_db.display()))?;
    storage.set_space(&space).await?;

    for req in &handles {
        let sub_label: Subname = req
            .handle
            .split('@')
            .next()
            .ok_or_else(|| anyhow!("invalid handle: {}", req.handle))?
            .parse()
            .map_err(|e| anyhow!("invalid subname in {}: {e}", req.handle))?;
        let spk = hex::decode(&req.script_pubkey)
            .with_context(|| format!("invalid script_pubkey for {}", req.handle))?;
        storage
            .add_handle(&sub_label.to_string(), &spk, None)
            .await?;
    }

    let (commitment_id, idx) = storage
        .add_commitment(None, &tip_root, &zk_batch, None)
        .await?;
    let committed = storage.commit_staged_handles(&tip_root, idx).await?;
    if committed != handles.len() {
        bail!(
            "expected to commit {} handles, committed {committed}",
            handles.len()
        );
    }

    if let Some(txid) = cli.commit_txid {
        storage
            .update_commitment_txid(commitment_id, &txid)
            .await?;
        println!("set commit_txid: {txid}");
    } else {
        eprintln!(
            "warning: no --commit-txid provided; subs may not treat the adopted commit as broadcast until you set it"
        );
    }

    // Sanity check: recomputed root after copy still matches.
    let copied_root = compute_sdb_root(&dest_sdb)?;
    if copied_root != tip_root {
        bail!("destination .sdb root mismatch after copy: {copied_root}");
    }

    let names: BTreeSet<_> = handles.iter().map(|h| h.handle.as_str()).collect();
    if names.len() != handles.len() {
        bail!("duplicate handle names in adoption set");
    }

    println!("seeded {}", dest_db.display());
    println!("adoption complete for {space}");
    Ok(())
}
