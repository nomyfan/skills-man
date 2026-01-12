mod cli;
mod errors;
mod models;
mod utils;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "skills-man")]
#[command(version)]
#[command(about = "Manage Agents skills")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Install {
        /// GitHub URL of the skill to install
        url: String,

        /// Force reinstall even if already installed
        #[arg(short, long)]
        force: bool,
    },
    Sync,
    Uninstall {
        /// Name of the skill to uninstall
        name: String,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Install { url, force } => cli::install_skill(&url, force),
        Commands::Sync => cli::sync_skills(),
        Commands::Uninstall { name } => cli::uninstall_skill(&name),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
