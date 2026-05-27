//! Decode a Bitcoin WIF private key to 64 hex characters (32-byte secret).

use anyhow::{Context, Result};
use bitcoin::PrivateKey;
use clap::Parser;

#[derive(Parser)]
#[command(
    name = "wif-to-hex",
    about = "Decode Bitcoin WIF to a 64-character hex private key"
)]
struct Cli {
    /// WIF private key (mainnet K/L/5 prefix, or testnet c/9 prefix)
    wif: String,

    /// Prefix output with 0x
    #[arg(long)]
    prefix: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let pk = PrivateKey::from_wif(cli.wif.trim())
        .context("invalid WIF (bad Base58Check encoding or checksum)")?;
    let hex = hex::encode(pk.inner.secret_bytes());
    if cli.prefix {
        print!("0x");
    }
    println!("{hex}");
    Ok(())
}
