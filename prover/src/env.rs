//! Environment variable and `.env` file loading.

use clap::ArgMatches;
use config_origins::{
    self as origins, origin_for_env_var, origin_from_clap, DotenvLoad,
};

pub use config_origins::load_dotenv;

/// Log effective `subs-prover` configuration for server mode.
pub fn log_server_startup(matches: &ArgMatches, dotenv: &DotenvLoad, server: bool, port: u16) {
    origins::log_section("subs-prover", dotenv);
    origins::log_entry(
        "server",
        server,
        origin_from_clap(matches, "server", Some("SUBS_PROVER_SERVER"), dotenv),
    );
    origins::log_entry(
        "server_port",
        port,
        origin_from_clap(matches, "server_port", Some("SUBS_PROVER_PORT"), dotenv),
    );
    println!("  server_url = http://127.0.0.1:{} (derived from server_port)", port);
}

/// Log configuration for a prove/compress subcommand.
pub fn log_subcommand_startup(
    sub: &ArgMatches,
    dotenv: &DotenvLoad,
    sub_name: &str,
    input: Option<&std::path::Path>,
    output: Option<&std::path::Path>,
) {
    origins::log_section("subs-prover", dotenv);
    println!("  command = {sub_name} (param)");

    log_io_path(sub, "input", "SUBS_PROVER_INPUT", input, dotenv);
    log_io_path(sub, "output", "SUBS_PROVER_OUTPUT", output, dotenv);
}

/// Log configuration for the bench subcommand.
pub fn log_bench_startup(dotenv: &DotenvLoad, sub: &ArgMatches, existing: usize, insert: usize) {
    origins::log_section("subs-prover", dotenv);
    println!("  command = bench (param)");
    origins::log_entry(
        "bench_existing",
        existing,
        origin_from_clap(sub, "existing", Some("SUBS_PROVER_BENCH_EXISTING"), dotenv),
    );
    origins::log_entry(
        "bench_insert",
        insert,
        origin_from_clap(sub, "insert", Some("SUBS_PROVER_BENCH_INSERT"), dotenv),
    );
}

fn log_io_path(
    sub: &ArgMatches,
    field_id: &str,
    env_var: &str,
    value: Option<&std::path::Path>,
    dotenv: &DotenvLoad,
) {
    let display = value.map(|p| p.display().to_string());
    let origin = match sub.value_source(field_id) {
        Some(_) => Some(origin_from_clap(sub, field_id, Some(env_var), dotenv)),
        None if display.is_some() => origin_for_env_var(env_var, dotenv),
        None => None,
    };
    origins::log_entry_optional(field_id, display.as_deref(), origin, false);
}
