# skills-man

`skills-man` is a Rust CLI that installs and manages AI agent skills from GitHub
repositories. It downloads skills into a local `skills/` directory and tracks
them in `skills.toml` so you can keep installs clean and up to date.

## Features

- Install skills from GitHub URLs.
- Sync local skills with upstream changes.
- Detect local modifications via directory checksums.
- List and uninstall skills.
- Update a single skill.
- Local and global modes.

## Quick start

Install the binary directly from Git:

```bash
cargo install --git https://github.com/nomyfan/skills-man
```

Run the CLI:

```bash
skill install https://github.com/owner/repo/tree/main/path/to/skill
skill list
skill sync
skill update skill-name
skill uninstall skill-name
```

Or build and run locally:

```bash
cargo build
cargo run -- install https://github.com/owner/repo/tree/main/path/to/skill
```

## Commands

`skill install <github-url>` (alias: `skill i`)
Install a skill from a GitHub URL.

`skill sync`
Sync all skills from `skills.toml`, downloading missing skills and optionally
overwriting local changes.

`skill uninstall <skill-name>`
Remove a skill directory and its entry in `skills.toml`.

`skill update <skill-name>` (alias: `skill up`)
Check for upstream changes and update a single skill.

`skill list`
Show installed skills and their metadata.

## Directory modes

By default, `skills-man` works in **local mode** and stores data in the current
directory:

- `./skills/`
- `./skills.toml`

Use `-g` / `--global` to switch to **global mode**:

- `~/.skills-man/skills/`
- `~/.skills-man/skills.toml`

Examples:

```bash
skill --global install https://github.com/owner/repo/tree/main/path/to/skill
skill -g list
```

## Use case: share skills across agent CLIs

If you use multiple agent CLIs (Codex, Claude Code, Gemini), install skills in
global mode and symlink each agent's skills directory to `~/.skills-man/skills`.
This keeps a single source of truth for all tools.

```bash
skill -g install https://github.com/owner/repo/tree/main/path/to/skill
skill -g install https://github.com/owner/repo/tree/main/path/to/another-skill

mkdir -p ~/.claude
mkdir -p ~/.codex
mkdir -p ~/.gemini

ln -s ~/.skills-man/skills ~/.claude/skills
ln -s ~/.skills-man/skills ~/.codex/skills
ln -s ~/.skills-man/skills ~/.gemini/skills
```

You can share `~/.skills-man/skills.toml` with teammates, run `skill -g sync`, and everyone gets the same skill set.

## GitHub URL format

Skills must be referenced with the GitHub "tree" URL that points at a directory(currently only GitHub URLs are supported):

```
https://github.com/<owner>/<repo>/tree/<ref>/<path>
```

`<ref>` can be a branch name, tag, or commit SHA. Both refs and paths can
contain slashes, so the tool tries multiple candidate splits until one succeeds.

Examples:

```
https://github.com/anthropics/skills/tree/main/skills/frontend-design
https://github.com/owner/repo/tree/release/v1.0/path/to/skill
```

## How syncing works

- Each installed skill is recorded in `skills.toml`.
- A SHA256 checksum of the skill directory detects local edits.
- On `sync`, if a checksum mismatch is found, you will be prompted before
  overwriting local changes.

During `install`, the tool resolves the ref to a commit SHA using the GitHub API
and only re-downloads when the upstream SHA changes.

## License

MIT
