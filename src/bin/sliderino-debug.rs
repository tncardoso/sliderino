//! Debug tooling for agents: subcommands that exercise parts of the app in
//! isolation (e.g. build a scene and render a screenshot) so outcomes can be
//! validated without driving the GUI.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "sliderino-debug", about = "Debug subcommands for testing sliderino internals")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Placeholder: confirms the debug binary runs.
    Ping,
}

fn main() {
    match Cli::parse().command {
        Command::Ping => println!("pong"),
    }
}
