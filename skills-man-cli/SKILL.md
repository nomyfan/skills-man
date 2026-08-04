---
name: skills-man-cli
description: Use the skills-man `skill` CLI to install, sync, update, list, and uninstall AI agent skills from GitHub repositories. Use when a user asks how to manage skills with skills-man, configure global or local skill directories, install a skill or collection from a GitHub tree URL, set GitHub authentication for private repositories, inspect skills.toml state, or troubleshoot common skills-man CLI behavior.
---

# Skills Man CLI

## Overview

Use `skill` to manage AI agent skills from GitHub. The CLI downloads skill folders into `skills/` and tracks install metadata in `skills.toml`.

## Directory Mode

Choose the directory mode based on the user's goal:

- Global mode: pass `-g` or `--global`, uses `~/.skills-man/skills/` and `~/.skills-man/skills.toml`.
- Local mode: omit `-g`, uses `./skills/` and `./skills.toml` in the current directory.

Use local mode for skill sets that should stay under the current working directory.

## GitHub URLs

Use GitHub `tree` URLs that point to a directory:

```text
https://github.com/<owner>/<repo>/tree/<ref>/<path>
```

`<ref>` may be a branch, tag, or commit SHA. Refs and paths may both contain slashes; the CLI tries candidate splits until it finds a valid directory.

Examples:

```bash
skill install https://github.com/owner/repo/tree/release/v1.0/path/to/skill
```

## Core Commands

Use `skill --help` or `skill <command> --help` to check the current command syntax.

Install one skill:

```bash
skill install https://github.com/owner/repo/tree/main/path/to/skill
```

Install every skill inside a collection directory. A collection is a directory whose immediate child directories contain `SKILL.md` files:

```bash
skill install https://github.com/owner/repo/tree/main/skills
skill install --yes https://github.com/owner/repo/tree/main/skills
```

Re-installing a collection (via `install` or `update --collection`) also detects skills that were removed from the collection upstream. It lists them and, after confirmation (or automatically with `--yes`), deletes their local directory and `skills.toml` entry.

List installed skills:

```bash
skill list
```

Sync all entries from `skills.toml`. Sync downloads missing skills and prompts before replacing directories whose checksums show local edits:

```bash
skill sync
skill --global sync
```

Update one installed skill by reusing its recorded source URL:

```bash
skill update skill-name
skill update --yes skill-name
```

Update the collection that originally installed a skill:

```bash
skill update --collection skill-name
```

This re-installs the collection, so skills removed upstream are also detected and offered for local removal (see collection install above).

Uninstall a skill:

```bash
skill uninstall skill-name
```

## Authentication

For higher GitHub API limits or private repositories, configure `GITHUB_TOKEN` or `GH_TOKEN` in the shell or in `~/.skills-man/config.toml`.

```toml
[env]
GITHUB_TOKEN = "github_pat_..."
```

Shell environment values take priority over `~/.skills-man/config.toml`.

## TLS Trust Store

`skill` trusts only the Mozilla root certificates bundled into the binary, not the operating system store. Where TLS is intercepted by a corporate or sandbox proxy, `curl` and `git` succeed while `skill` fails with:

```text
TLS certificate verification failed

Reason: io: invalid peer certificate: UnknownIssuer
```

Widening trust is the user's decision. Report the error and the options; do not export these variables on the user's behalf:

- `SKILLS_MAN_TLS_ROOTS=system` trusts the operating system store, like `curl` and `git`.
- `SSL_CERT_FILE=/path/ca-bundle.pem` or `SSL_CERT_DIR=/path/certs` trusts specific CAs.

Never combine `SKILLS_MAN_TLS_ROOTS` with `SSL_CERT_FILE` or `SSL_CERT_DIR`; the CLI rejects that combination rather than ranking them. An empty value still counts as set, so `SSL_CERT_FILE=` conflicts too.

These options apply only when the reason mentions `UnknownIssuer`. A `TLS certificate verification failed` error without it may mean the server certificate is expired, not yet valid, issued for another host, or is otherwise invalid; changing the trust store does not help there.

## Troubleshooting

Use `skill list` to confirm install metadata before update or uninstall.

Use `skill sync` when `skills.toml` exists and skill directories are missing.

Use `skill update --collection <skill-name>` when a skill came from a collection and the user expects sibling skills to update too.

Use `--yes` for non-interactive install and update flows.

Use a full `https://github.com/.../tree/.../...` URL when install reports an invalid URL.

Set `GITHUB_TOKEN` or `GH_TOKEN` when GitHub returns rate-limit, forbidden, or private repository access errors.

Present the trust store options (see TLS Trust Store) when a command fails with `TLS certificate verification failed` **and** the reason mentions `UnknownIssuer`; checking that `curl` reaches the same host confirms basic connectivity and points to a certificate-chain or trust-configuration difference. For any other certificate failure, check the system clock, the host name, and whether something is rewriting the connection instead.
