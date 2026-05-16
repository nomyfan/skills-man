mod cli;
mod errors;
mod models;
mod providers;
mod utils;

use crate::models::AppConfig;
use clap::{Parser, Subcommand};
use providers::{ProviderRegistry, github::GitHubProvider};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::PathBuf,
};
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
    /// Install a skill or skill collection from GitHub
    #[command(visible_alias = "i")]
    Install {
        /// GitHub URL of the skill to install
        url: String,
        /// Automatically answer yes to prompts (non-interactive mode)
        #[arg(short, long)]
        yes: bool,
    },
    /// Sync installed skills from skills.toml
    Sync,
    #[command(visible_alias = "up")]
    /// Check upstream and update a skill
    Update {
        /// Name of the skill to update
        name: String,
        /// Update the collection containing this skill
        #[arg(short = 'c', long)]
        collection: bool,
        /// Automatically answer yes to prompts (non-interactive mode)
        #[arg(short, long)]
        yes: bool,
    },
    /// Remove an installed skill
    #[command(visible_alias = "rm")]
    Uninstall {
        /// Name of the skill to uninstall
        name: String,
    },
    /// List all installed skills
    #[command(visible_alias = "ls")]
    List,
}

fn get_global_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(|home| PathBuf::from(home).join(".skills-man"))
}

fn get_base_dir(global: bool) -> Result<PathBuf, String> {
    if global {
        get_global_dir().ok_or_else(|| "Unable to determine home directory".to_string())
    } else {
        Ok(PathBuf::from("."))
    }
}

fn load_config_env(config: &AppConfig) {
    let protected_env = std::env::vars_os()
        .map(|(key, _)| key)
        .collect::<HashSet<_>>();
    let mut merged_env = HashMap::new();

    for (key, value) in &config.env {
        let key_os = OsString::from(key.as_str());
        if protected_env.contains(&key_os) {
            continue;
        }
        merged_env.insert(key.clone(), value.clone());
    }

    for (key, value) in merged_env {
        // SAFETY: this runs during single-threaded CLI startup, before any
        // worker threads or foreign-library calls can read the environment.
        unsafe { std::env::set_var(key, value) };
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

    let app_config = match get_global_dir() {
        Some(global_dir) => match AppConfig::from_file(global_dir.join("config.toml")) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("Warning: {e}");
                AppConfig::default()
            }
        },
        None => AppConfig::default(),
    };
    load_config_env(&app_config);

    let registry = match GitHubProvider::new() {
        Ok(github) => ProviderRegistry::new(vec![Box::new(github)]),
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };

    let result = match cli.command {
        Commands::Install { url, yes } => cli::install_skill(&url, &base_dir, yes, &registry),
        Commands::Sync => cli::sync_skills(&base_dir, &registry),
        Commands::Update {
            name,
            collection,
            yes,
        } => {
            if collection {
                cli::update_collection_for_skill(&name, &base_dir, yes, &registry)
            } else {
                cli::update_skill(&name, &base_dir, yes, &registry)
            }
        }
        Commands::Uninstall { name } => cli::uninstall_skill(&name, &base_dir),
        Commands::List => cli::list_skills(&base_dir),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
