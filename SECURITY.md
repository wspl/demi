# Security policy

This policy covers the supported versions, how to report a vulnerability, how
operators should handle data, and the sandboxing of Clouds and devices.

## Supported versions

Demi is pre-1.0. Security fixes land on the latest release. There are no
backported maintenance branches yet.

## Reporting a vulnerability

Report security issues **privately**. Don't open a public issue or PR that
discloses the problem.

Report through either channel:

- GitHub's private security advisory on this repository ("Report a
  vulnerability" under Security)
- Email to [support@demi.sh](mailto:support@demi.sh)

Include a description, affected versions or packages, and a reproduction if
possible.

We aim to acknowledge reports promptly and coordinate a fix and disclosure.

## Data handling notes for operators

Demi is a hosted product. The backend keeps the product's records, and each
conversation's work runs on a Host: the user's Cloud or a device the user
paired.

Operators handling sensitive prompts should know where data rests:

- **Backend storage.** Conversation records, transcripts and uploaded media
  live in the backend's data directory and object store
  (`crates/backend-database/src/lib.rs`).
  - The backend stores passwords as argon2id hashes.
  - The backend stores session and device tokens as SHA-256 digests.
  - The backend seals provider credentials with keys derived from the instance
    secret (`crates/backend/src/config/secret.rs`).
- **Hosts.**
  - A job's full output stays in files on its Host, and the backend receives
    bounded views (pipes and output: `crates/runner-jobs/src/tasks.rs`).
  - Each runner keeps a Host log of its own and its services' diagnostics in
    its data directory (Host log: `crates/runner/src/host_log.rs`).
- **Claude Code.**
  - The selected account's token reaches the Claude Code CLI on the user's
    Cloud through the process environment, and never reaches a paired device
    ([Data and credentials](docs/overview.md#data-and-credentials)).
  - Demi writes the raw stream-json exchange, prompts included, only to a
    `tracing` target that is off by default
    (`crates/provider-claude-code/src/live.rs`).
- **Failure records.**
  - A failure record keeps only what the vendor sent back, never the request
    Demi sent, so it never contains a credential Demi used.
  - Demi shows vendor error messages as the vendor sent them, without
    redaction (`crates/shared-types/src/failure.rs`).

## Sandboxing

Each user's Cloud is a gVisor sandbox on a Linux host, with its own gVisor
kernel, filesystem view, network namespace and resource group.

The sandbox is the isolation boundary. The infrastructure still trusts the
Linux kernel, the machine manager, the runsc distribution and the image builder
(`crates/machine-manager/src/sandbox/oci.rs`).

A user's projects share that user's Cloud. Jobs run as the image's `demi` user,
which has passwordless sudo inside the sandbox
(`cloud-guest-image/rootfs/build.sh`).

A paired device runs the runner with the permissions of its local account. The
device executes the requests the backend authorizes, so the backend is part of
that device's execution trust boundary
([A conversation using a device](docs/overview.md#a-conversation-using-a-device)).
