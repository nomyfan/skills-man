use std::io::{self, IsTerminal, Write as IoWrite};

fn confirm_action_inner(prompt: &str, non_interactive_message: &str) -> bool {
    if !io::stdin().is_terminal() {
        eprintln!("{}", non_interactive_message);
        return false;
    }

    print!("{} (y/N): ", prompt);
    io::stdout().flush().ok();

    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    let answer = input.trim().to_lowercase();

    answer == "y" || answer == "yes"
}

/// Prompt user for confirmation in interactive mode.
/// Returns false when stdin is not a TTY.
pub fn confirm_action(prompt: &str) -> bool {
    confirm_action_inner(prompt, "Non-interactive mode detected. Defaulting to no.")
}

/// Prompt user for confirmation, unless `yes` is set.
pub fn confirm_action_or_yes(prompt: &str, yes: bool) -> bool {
    if yes {
        return true;
    }

    confirm_action_inner(
        prompt,
        "Non-interactive mode detected. Use --yes flag to auto-confirm.",
    )
}
