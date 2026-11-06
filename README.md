# skills-man

`skills-man` is a Rust CLI that installs and manages AI agent skills from GitHub
repositories. It downloads skills into a local `skills/` directory and tracks
them in `skills.toml` so you can keep installs clean and up to date.

## Features

- Install a single skill or a skill collection from GitHub (currently only GitHub URLs are supported).
- Sync local skills with upstream changes.
- List, update and uninstall skills.
- Local and global modes.
- Load environment variables from `~/.skills-man/config.toml`.

## Quick start

Install the latest release with the install script.

macOS / Linux (installs to `~/.local/bin`):

```bash
curl -fsSL https://raw.githubusercontent.com/nomyfan/skills-man/main/install.sh | sh
```

Windows PowerShell (installs to `~\.local\bin` and adds it to your user `PATH`):

```powershell
irm https://raw.githubusercontent.com/nomyfan/skills-man/main/install.ps1 | iex
```

Set `SKILLS_MAN_INSTALL_DIR` to install somewhere else, for example
`curl -fsSL .../install.sh | SKILLS_MAN_INSTALL_DIR=~/bin sh`.
Running the script again upgrades `skill` to the latest release.

Or download a prebuilt binary from GitHub [Releases](https://github.com/nomyfan/skills-man/releases),
extract it, rename it to `skill` (`skill.exe` on Windows) and place it in a
directory on your `PATH`.

Or build and install from source:

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

## Environment variables

At startup, `skill` loads environment variables from the global config file.
Priority is:

1. Existing shell environment variables
2. `~/.skills-man/config.toml` `[env]` section

Set `GITHUB_TOKEN` or `GH_TOKEN` in the global config:

```toml
# ~/.skills-man/config.toml
[env]
GITHUB_TOKEN = "github_pat_..."
```

The token is sent with GitHub API requests so `skill` can use authenticated rate
limits and access private repositories that your token is allowed to read.

## TLS trust store

By default `skill` trusts only the Mozilla root certificates bundled into the
binary. The same build then trusts the same CAs on every machine, and a proxy
that terminates TLS cannot sit in the middle unnoticed.

That default fails where TLS *is* intercepted — many corporate networks, CI
sandboxes and cloud dev containers install their proxy's CA into the operating
system trust store, so `curl` and `git` work while `skill` fails with
`invalid peer certificate: UnknownIssuer`.

If you trust that proxy, widen the trust store explicitly:

```bash
# The operating system store, like curl and git use
SKILLS_MAN_TLS_ROOTS=system skill install https://github.com/owner/repo/tree/main/path/to/skill

# Or specific CAs: a PEM bundle, directories of PEM files, or both together
SSL_CERT_FILE=/path/to/ca-bundle.pem skill sync
SSL_CERT_DIR=/etc/ssl/certs:/usr/local/share/ca-certificates skill sync
```

`SKILLS_MAN_TLS_ROOTS` accepts `system` or `webpki` (the default), and cannot be
combined with either CA variable: on unix `system` resolves its roots through
`rustls-native-certs`, which reads those variables *instead of* the operating
system store whenever either is set, so ranking them would mean one thing on
Linux and another on macOS. For the same reason an **empty** value still counts
as set.

Trust is never widened implicitly. A CA configuration that yields no usable trust
anchors, an unknown `SKILLS_MAN_TLS_ROOTS` value, and a certificate that fails to
verify are all hard errors rather than a quiet fallback to the bundled roots.

These variables can also live in `~/.skills-man/config.toml`:

```toml
# ~/.skills-man/config.toml
[env]
SKILLS_MAN_TLS_ROOTS = "system"
```

## GitHub API mirror

All requests go to `https://api.github.com` by default. To route them through a
mirror or reverse proxy, set
`SKILLS_MAN_GITHUB_API_URL`. The value is used as a plain prefix for
`/repos/...` paths, so any proxy layout works:

```bash
SKILLS_MAN_GITHUB_API_URL=https://proxy.example.com/api.github.com skill sync
SKILLS_MAN_GITHUB_API_URL=https://proxy.example.com/https://api.github.com skill sync
```

Skill URLs passed to `skill install` and stored in `skills.toml` keep the
`https://github.com/...` form. Note that `GITHUB_TOKEN`/`GH_TOKEN`, when set,
is sent to the configured host. Archive downloads hit
`/repos/{owner}/{repo}/tarball/{sha}`, which GitHub answers with a redirect to
`codeload.github.com`; a proxy that passes that redirect through unchanged sends
the download around the proxy.

Like the other variables, it can also be set in the `[env]` section of
`~/.skills-man/config.toml`.

## Commands

`skill install <github-url>` (alias: `skill i`)
Install a skill or a skill collection from GitHub.

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

## Agent skill

This repository includes an Agent skill at `skills-man-cli/`. Install it so an
AI agent can use and troubleshoot the `skill` CLI:

```bash
skill -g install https://github.com/nomyfan/skills-man/tree/main/skills-man-cli
```

## GitHub URL format

Skills must be referenced with the GitHub "tree" URL that points at a directory:

```
https://github.com/<owner>/<repo>/tree/<ref>/<path>
```

`<ref>` can be a branch name, tag, or commit SHA. Both refs and paths can
contain slashes, so the tool tries multiple candidate splits until one succeeds.

Examples:

```
https://github.com/nomyfan/skills-man/tree/main/skills-man-cli
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
