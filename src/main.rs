mod cli;
mod errors;
mod models;
mod utils;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "skills-man")]
#[command(version)]
#[command(about = "Manage Agents skills")]
struct Cli {
    /// Use global directory (~/.skills-man)
    #[arg(short, long, global = true)]
    global: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(alias = "i")]
    Install {
        /// GitHub URL of the skill to install
        url: String,
    },
    Sync,
    #[command(alias = "up")]
    /// Check upstream and update a single skill
    Update {
        /// Name of the skill to update
        name: String,
    },
    Uninstall {
        /// Name of the skill to uninstall
        name: String,
    },
    /// List all installed skills
    List,
}

fn get_base_dir(global: bool) -> Result<PathBuf, String> {
    if global {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map_err(|_| "Unable to determine home directory".to_string())?;
        Ok(PathBuf::from(home).join(".skills-man"))
    } else {
        Ok(PathBuf::from("."))
    }
}

fn main() {
    let cli = Cli::parse();

    let base_dir = match get_base_dir(cli.global) {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    let result = match cli.command {
        Commands::Install { url } => cli::install_skill(&url, &base_dir),
        Commands::Sync => cli::sync_skills(&base_dir),
        Commands::Update { name } => cli::update_skill(&name, &base_dir),
        Commands::Uninstall { name } => cli::uninstall_skill(&name, &base_dir),
        Commands::List => cli::list_skills(&base_dir),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
