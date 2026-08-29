mod bomb;
mod cli;
mod codec;
mod commands;
mod config;
mod crypto;
mod dedup;
mod exclude;
mod foreign;
mod format;
mod progress;
mod util;

use clap::Parser;

use cli::{Cli, Command};

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Compress(args) => exit_on_result(commands::compress::run(args)),
        Command::Decompress(args) => exit_on_result(commands::decompress::run(args)),
        Command::Shred(args) => exit_on_result(commands::shred::run(args)),
        Command::Info(args) => exit_on_result(commands::info::run(args)),
        Command::Dedupe(args) => exit_on_result(commands::dedupe::run(args)),
        // `diff` and `search` follow the classic grep/diff exit-code
        // contract (0 = found/identical, 1 = none found/differs, 2 =
        // trouble), so they share the tri-state exit path below instead of
        // the plain success/failure one the other commands use.
        Command::Diff(args) => exit_on_match_result(commands::diff::run(args)),
        Command::Search(args) => exit_on_match_result(commands::search::run(args)),
        Command::Completions(args) => exit_on_result(commands::completions::run(args)),
        Command::Config(args) => exit_on_result(commands::config::run(args.action)),
    }
}

fn exit_on_result(result: anyhow::Result<()>) -> ! {
    match result {
        Ok(()) => std::process::exit(0),
        Err(err) => {
            eprintln!("error: {err:#}");
            std::process::exit(1);
        }
    }
}

/// For commands whose success has two distinct flavors (diff: identical vs.
/// differs; search: matched vs. no matches): `Ok(true)` exits 0, `Ok(false)`
/// exits 1, and a genuine error exits 2.
fn exit_on_match_result(result: anyhow::Result<bool>) -> ! {
    match result {
        Ok(true) => std::process::exit(0),
        Ok(false) => std::process::exit(1),
        Err(err) => {
            eprintln!("error: {err:#}");
            std::process::exit(2);
        }
    }
}
