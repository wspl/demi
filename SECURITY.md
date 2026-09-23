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

Demi is a hosted product. The backend keeps the product's records, and each
conversation's work runs on a Host: the user's Cloud or a device the user
paired. Operators handling sensitive prompts should know where data rests:

- **Backend storage.** Conversation records, transcripts and uploaded media
  live in the backend's data directory and object store
  ([Storage](docs/backend/storage.md)). Passwords are stored as argon2id
  hashes, session and device tokens as SHA-256 digests, and provider
  credentials sealed with keys derived from the instance secret
  ([Passwords and credentials at rest](docs/backend/storage.md#passwords-and-credentials-at-rest)).
- **Hosts.** A job's full output stays in files on its Host, and the backend
  receives bounded views
  ([Pipes and output](docs/execution/runner.md#pipes-and-output)). Each runner
  keeps a Host log of its own and its services' diagnostics in its data
  directory ([Host log](docs/execution/runner.md#host-log)).
- **Claude Code.** The selected account's token reaches the Claude Code CLI on
  the user's Cloud through the process environment, and never reaches a paired
  device ([Data and credentials](docs/overview.md#data-and-credentials)). The
  raw stream-json exchange, prompts included, is written only to a `tracing`
  target that is off by default
  ([Process lifetime](docs/providers/claude-code.md#process-lifetime)).
- **Failure records.** A failure record keeps only what the vendor sent back,
  never the request Demi sent, so it never contains a credential Demi used.
  Vendor error messages are shown as the vendor sent them, without redaction
  ([The failure record](docs/agent/failures-and-recovery.md#the-failure-record)).

## Sandboxing

Each user's Cloud is a gVisor sandbox on a Linux host, with its own gVisor
kernel, filesystem view, network namespace and resource group. The sandbox is
the isolation boundary; the infrastructure still trusts the Linux kernel, the
machine manager, the runsc distribution and the image builder
([Isolation and joining](docs/cloud/managed-hosts.md#isolation-and-joining)).
A user's projects share that user's Cloud: jobs run as the image's `demi` user,
which has passwordless sudo inside the sandbox
([Cloud images](docs/cloud/images.md)).

A paired device runs the runner with the permissions of its local account. The
device executes the requests the backend authorizes, so the backend is part of
that device's execution trust boundary
([A conversation using a device](docs/overview.md#a-conversation-using-a-device)).

An expose makes a service on a Host reachable at an unguessable public URL for
a limited time, and anyone who has the URL reaches the service
([Expose](docs/execution/expose.md)).
