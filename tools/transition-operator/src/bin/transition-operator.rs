use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "transition-operator")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Version,
}

fn main() {
    match Cli::parse().command {
        Command::Version => println!(
            "transition-operator {} ({})",
            transition_operator::TOOL_VERSION,
            transition_operator::PLAN_SCHEMA_VERSION
        ),
    }
}
