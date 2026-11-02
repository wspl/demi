---
status: proposed
---

# An agent saves a credential from a pipe

## Problem

Alex asks an agent to run model tests with the API key in
`~/Projects/acme/.env`. The agent wants to keep the key as a credential, so
that later commands pass it on with `demi credentials read` instead of reading
the file. Its only way to save one is `demi credentials request`, a card on
which Alex pastes the values. So the agent shows Alex a card that says where
the key is and asks Alex to open the file, copy the key and paste it, for a
value the agent could read itself.

The agent cannot pass the value as an argument either. The shell expands
`--value "$(grep … .env)"` before Demi sees it, and Demi shows and stores the
expanded command line on a permission card: the value would reach the page and
the database.

## Decision

`demi credentials save` saves a credential from values piped on its standard
input, which no model reads:

```sh
grep '^ACME_API_KEY=' .env | cut -d= -f2- \
  | demi credentials save "Acme API key" --field api_key:secret:"API key"
```

- `save <name> --field … [--reason …] [--keep …]` takes the same arguments as
  `request`.
- Its standard input holds the values: for a credential of one field, the
  value as it is, without one trailing newline, so a key file or a PEM block
  pipes in whole; for several fields, one `<field>=<value>` line each, as
  `gh secret set --env-file` reads them. A message about the input names the
  line, never what it holds.
- It asks the user nothing. A credential of a new name is saved; one of a
  saved name and the same fields gets the new values, as a renewal does. Either
  is then allowed in the conversation, as one the user saved from a request
  is, and the command prints what `request` prints. A saved name with other
  fields fails, as `request` does.
- No page changes: the credential shows in Settings › Credentials as one the
  user saved does.

### Design

- `backend-user-shard` `credential_commands.rs`: the `save` leaf, with
  `stdin_field("values")`, parses the values against the fields and calls
  `backend_permissions::save_credential`.
- `backend-database` `ControlDatabase::save_credential(conversation, name,
  ask, values, seal)` in one transaction saves or renews the credential and
  allows it, or answers `FieldsDiffer(fields)` and writes nothing. It shares
  that step with `answer_credential`, which saves a request's typed values.
- The group's index entry says an agent can save one from a pipe.

## Spike

None.

## Prior art

- GitHub CLI: `gh secret set NAME` reads the value from standard input, and
  `--env-file` reads `NAME=value` lines
  ([docs](https://cli.github.com/manual/gh_secret_set)).
- Wrangler `secret put` and `docker secret create NAME -` read the value from
  standard input.
- 1Password CLI `op item create` and `op item edit` add and change an item
  without a prompt.

## Alternatives

- A card with the values filled in, which the user saves: one more click for
  each save, and the secret goes to the page.
- Asking before a replacement of a saved credential's values: a card that
  cannot show the secret is approved blind, and it guards nothing the agent
  could not do with the value directly.
- Reading a file named in an argument on the Host: a second way to the same
  value, with paths and formats to support; a pipe already reaches any
  file.
- Values as arguments: the expanded command line is shown and stored.

## Open questions

None.
