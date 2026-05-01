mod cli;
mod errors;
mod models;
mod utils;

use clap::{Parser, Subcommand};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(name = "skills-man")]
#[command(version("0.1.1"))]
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
    /// Check upstream and update a single skill
    Update {
        /// Name of the skill to update
        name: String,
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

fn merge_env_file(
    path: &Path,
    protected_env: &HashSet<OsString>,
    merged_env: &mut HashMap<String, String>,
) {
    let Ok(iter) = dotenvy::from_path_iter(path) else {
        return;
    };

    for item in iter {
        let Ok((key, value)) = item else {
            continue;
        };

        let key_os = OsString::from(&key);
        if protected_env.contains(&key_os) {
            continue;
        }

        merged_env.insert(key, value);
    }
}

fn load_env_files() {
    let protected_env = std::env::vars_os()
        .map(|(key, _)| key)
        .collect::<HashSet<_>>();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut merged_env = HashMap::new();

    if let Some(global_dir) = get_global_dir() {
        merge_env_file(&global_dir.join(".env"), &protected_env, &mut merged_env);
    }
    merge_env_file(&cwd.join(".env"), &protected_env, &mut merged_env);

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

    load_env_files();

    let result = match cli.command {
        Commands::Install { url, yes } => cli::install_skill(&url, &base_dir, yes),
        Commands::Sync => cli::sync_skills(&base_dir),
        Commands::Update { name, yes } => cli::update_skill(&name, &base_dir, yes),
        Commands::Uninstall { name } => cli::uninstall_skill(&name, &base_dir),
        Commands::List => cli::list_skills(&base_dir),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
