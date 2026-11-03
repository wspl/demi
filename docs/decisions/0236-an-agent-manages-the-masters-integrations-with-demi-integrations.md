---
status: proposed
---

# An agent manages the master's integrations with `demi integrations`

## Problem

Alex, the server's master, asks an agent to set up an integration for a
support bot it is writing: a name, subagents and the browser off, an init
script that installs the bot's tools, and 4 GB of memory for each Cloud.
Every other setting the agent can change has a `demi` command, such as
`demi providers` for the provider entries, but an integration has only
Settings › Integrations, so Alex does it by hand while the agent waits, and
copies the token out of the page into the bot's configuration.

## Decision

A `demi integrations` group does what Settings › Integrations does, in the
master's conversations only:

| Command | What it does |
| --- | --- |
| `list` | Each integration: id, name, Clouds, conversations. A read. |
| `show <integration>` | Every setting the page has: name, capabilities, init script, shared folder, idle minutes, CPUs, memory and disks. With `--json`, the object `edit` changes, whose `tokenFingerprint` names which token is current. Never the token. A read. |
| `add` | A new integration with the page's defaults, named *Integration*, *Integration 2* for the second. |
| `edit <integration>` | A JSON merge patch on stdin of what `show --json` prints: a field given takes its value, one left out stays, `null` sets a setting back to the server's. |
| `token <integration>` | A new token; the old one stops at once. |
| `remove <integration>` | Deletes it with its conversations, Clouds and shared folder, as Delete Integration does. Only when the user asks for it. |

* An integration is named by its name or its id, as `demi providers` names
  an entry.
* **The token never reaches the model.** `add` and `token` save it in the
  master's vault and print the credential's name and its card's tag
  (0289). The agent shows the card to the user, or pipes the token on with
  `demi credentials read`. The token goes into the credential that holds
  the integration's token, one secret field `token`, found by the token's
  hash whatever the credential or the integration was renamed; without
  one, into a new credential *<name> token*, or *<name> token 2* when the
  user has a credential of that name, which is never overwritten. The
  credential is chosen before the token is made. `remove` removes that
  credential too. A credential with other fields, or a token the user
  changed, stays the user's.
* **One permission.** Every command but `list` and `show` needs
  `integrations.manage`, "manage integrations": "Add, change and remove
  your integrations, their tokens and the sizes of their Clouds." A grant
  lasts as other grants do; `remove`'s help says it runs only when the user
  asks, as `demi credentials remove`'s does.
* **Only the master's conversations have it.** A conversation of another
  account has no integrations (0227), and an integration's own
  conversations never have the group, as they never have the vault.

The handlers call the shard's integration methods the web routes call
(`crates/backend-user-shard/src/integrations.rs`), so a setting means the
same on the page and in the command. Settings › Integrations follows the
integrations on the synchronization channel, as Settings › Credentials
follows the credentials, so an open page shows an agent's change at once;
only the master's channels carry them.

## Spike

None: the shard's methods take the web's `IntegrationPatch`, and the vault
saves a credential from the backend as `demi credentials save` does.

## Prior art

* `demi providers` lists, shows, adds, edits by merge patch and removes
  provider entries under one permission; this group follows it.
* GitHub's `gh` prints a token only on request (`gh auth token`); here the
  token is never printed and goes to the vault instead.

## Alternatives

* **Print the token, as the page shows it once**: it reaches the model's
  vendor and stays in the transcript.
* **A confirmation card for every `remove`**: Demi has no card that asks
  for one command's run; the permission and the help's rule are what
  `demi credentials remove` and `demi providers remove` have.

## Open questions

None.
