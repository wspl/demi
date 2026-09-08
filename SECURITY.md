# Security Policy

## Supported versions

Demi is pre-1.0. Security fixes land on the latest release; there are no
backported maintenance branches yet.

## Reporting a vulnerability

Please report security issues **privately** — do not open a public issue or PR
that discloses the problem. If the repository is on GitHub, use a private security
advisory ("Report a vulnerability"); otherwise contact the maintainers directly.
Include a description, affected versions/packages, and a reproduction if possible.
We aim to acknowledge reports promptly and coordinate a fix and disclosure.

## Data handling notes for operators

Demi is a local agent toolkit; a few components write diagnostics to the local
filesystem. None of this data leaves the machine, but operators handling sensitive
prompts should be aware of it:

- **Claude Code wire log (default on).** The `@demicodes/provider-claude-code` transport
  records the raw provider request/response stream — which includes full prompt
  content — to `$TMPDIR/demi-claude-wire/claude-<session>.jsonl`. Disable it with
  `DEMI_CLAUDE_WIRE_LOG=0`, or relocate it with `DEMI_CLAUDE_WIRE_LOG_DIR`.
- **Secrets in errors.** Provider adapters redact known API keys from error
  messages (`redactSecretText`), but treat logs as potentially sensitive.

## Sandboxing

The backend uses `@demicodes/host-remote` for both managed Cloud and connected
runners. Managed Cloud isolates users in separate VMs; projects belonging to one
user share that user's machine and filesystem permissions. The runner performs
file operations and starts processes as the same guest user. A connected runner
has the permissions of its local account. See
[managed hosts](docs/demi-next/managed-hosts.md) and the
[package contract](docs/package-boundaries.md).
