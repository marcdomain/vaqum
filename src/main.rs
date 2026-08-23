mod cli;
mod codec;
mod commands;
mod dedup;
mod format;
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
        // `diff` follows the classic Unix exit-code contract (0 identical,
        // 1 differences found, 2 trouble), so it gets its own handling
        // instead of the uniform success/failure path the other commands use.
        Command::Diff(args) => match commands::diff::run(args) {
            Ok(true) => std::process::exit(0),
            Ok(false) => std::process::exit(1),
            Err(err) => {
                eprintln!("error: {err:#}");
                std::process::exit(2);
            }
        },
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
