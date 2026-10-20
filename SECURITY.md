# Security Policy

## Supported versions

Demi is pre-1.0. Security fixes land on the latest release; there are no
backported maintenance branches yet.

## Reporting a vulnerability

Please report security issues **privately** — do not open a public issue or PR
that discloses the problem. Use GitHub's private security advisory on this
repository ("Report a vulnerability" under Security), or write to
[support@demi.sh](mailto:support@demi.sh).
Include a description, affected versions/packages, and a reproduction if possible.
We aim to acknowledge reports promptly and coordinate a fix and disclosure.

## Data handling notes for operators

Demi is a hosted product. The backend keeps the product's records, and each
conversation's work runs on a Host: the user's Cloud or a device the user
paired. Operators handling sensitive prompts should know where data rests:

- **Backend storage.** Conversation records, transcripts and uploaded media
  live in the backend's data directory and object store
  (`crates/backend-database/src/lib.rs`). Passwords are stored as argon2id
  hashes, session and device tokens as SHA-256 digests, and provider
  credentials sealed with keys derived from the instance secret
  (`crates/backend/src/config/secret.rs`).
- **Hosts.** A job's full output stays in files on its Host, and the backend
  receives bounded views
  (Pipes and output (`crates/runner-jobs/src/tasks.rs`)). Each runner
  keeps a Host log of its own and its services' diagnostics in its data
  directory (Host log (`crates/runner/src/host_log.rs`)).
- **Claude Code.** The selected account's token reaches the Claude Code CLI on
  the user's Cloud through the process environment, and never reaches a paired
  device ([Data and credentials](docs/overview.md#data-and-credentials)). The
  raw stream-json exchange, prompts included, is written only to a `tracing`
  target that is off by default
  (`crates/provider-claude-code/src/live.rs`).
- **Failure records.** A failure record keeps only what the vendor sent back,
  never the request Demi sent, so it never contains a credential Demi used.
  Vendor error messages are shown as the vendor sent them, without redaction
  (`crates/shared-types/src/failure.rs`).

## Sandboxing

Each user's Cloud is a gVisor sandbox on a Linux host, with its own gVisor
kernel, filesystem view, network namespace and resource group. The sandbox is
the isolation boundary; the infrastructure still trusts the Linux kernel, the
machine manager, the runsc distribution and the image builder
(`crates/machine-manager/src/sandbox/oci.rs`).
A user's projects share that user's Cloud: jobs run as the image's `demi` user,
which has passwordless sudo inside the sandbox
(`cloud-guest-image/rootfs/build.sh`).

A paired device runs the runner with the permissions of its local account. The
device executes the requests the backend authorizes, so the backend is part of
that device's execution trust boundary
([A conversation using a device](docs/overview.md#a-conversation-using-a-device)).
