//! Print the Merkle root of a SpaceDB `.sdb` file.
//!
//! Example:
//!   cargo run -p sdb-root -- data_subspacer/eurt/@eurt.sdb

use anyhow::{Context, Result};
use clap::Parser;
use spacedb::db::Database;
use spacedb::{Configuration, Sha256Hasher};

#[derive(Parser)]
#[command(about = "Print the Merkle root of a SpaceDB .sdb file")]
struct Cli {
    /// Path to the `.sdb` file
    path: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Configuration::standard();
    let db = Database::<Sha256Hasher>::open_with_config(&cli.path, config)
        .with_context(|| format!("could not open SpaceDB file: {}", cli.path))?;

    let mut snap = db
        .begin_read()
        .context("could not begin read transaction")?;
    let root = snap
        .compute_root()
        .context("could not compute Merkle root")?;

    println!("{}", hex::encode(root));
    Ok(())
}
