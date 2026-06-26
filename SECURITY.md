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

- **Claude Code wire log (default on).** The `@demi/provider-claude-code` transport
  records the raw provider request/response stream — which includes full prompt
  content — to `$TMPDIR/demi-claude-wire/claude-<session>.jsonl`. Disable it with
  `DEMI_CLAUDE_WIRE_LOG=0`, or relocate it with `DEMI_CLAUDE_WIRE_LOG_DIR`.
- **Host-local store.** `@demi/host-local` persists command artifacts under
  `$TMPDIR/demi-host-local-store/`.
- **Secrets in errors.** Provider adapters redact known API keys from error
  messages (`redactSecretText`), but treat logs as potentially sensitive.

## Sandboxing

The shell runs against a `Host` abstraction. When running untrusted agents, supply
a `Host` that enforces your sandbox (path jail, restricted `spawn`, ephemeral
store) — see [docs/guides/implement-a-host.md](docs/guides/implement-a-host.md).
The default `@demi/host-local` grants full local filesystem and process access.
