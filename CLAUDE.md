# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

`skills-man` is a Rust CLI tool for managing AI agent skills from GitHub repositories. It downloads skills to a local `skills/` directory and tracks them in `skills.toml`.

**Directory modes**:

- **Local mode** (default): Uses `./skills/` and `./skills.toml` in the current directory
- **Global mode** (`-g`/`--global`): Uses `~/.skills-man/skills/` and `~/.skills-man/skills.toml`

## Commands

```bash
# Build and run
cargo build
cargo run -- install <github-url>
cargo run -- sync
cargo run -- uninstall <skill-name>

# Testing
cargo test
cargo test test_name

# Code quality
cargo check
cargo clippy
cargo fmt
```

## Architecture

**Binary name**: `skill` (defined in Cargo.toml)

**Main flow**: `main.rs` → `cli.rs` (command implementations) → `models.rs` (data structures) + `utils.rs` (helpers)

**Key concept**: GitHub URLs like `https://github.com/owner/repo/tree/release/v1.0/path/skill` are ambiguous because both refs and paths can contain slashes. The tool generates multiple candidate split points (ref=`release/v1.0` path=`path/skill` OR ref=`release` path=`v1.0/path/skill`) and tries each until one succeeds.

**Update detection**: During install, refs are resolved to commit SHAs via GitHub API. Skills are only re-downloaded if the upstream SHA has changed, preventing unnecessary downloads while detecting updates.

**Integrity**: Directory checksums (SHA256 of all files) detect local modifications during sync operations.
