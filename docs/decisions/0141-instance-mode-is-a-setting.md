---
status: proposed
---

# Whether users share the master's providers is a setting

## Problem

Alex installs Demi for a small team. `demi-server setup` refuses to go on
until Alex picks `shared` or `isolated`, a word the installer does not
explain, and the choice is fixed in the server's configuration file: to
change it, Alex edits the file and restarts the backend. On a shared
instance every user also runs on Alex's personal subscriptions, which
their vendors' terms may not allow.

Separately, the warning before a second account or a rotation names no risk
the user takes: it says that several subscriptions *may be against the
vendor's terms*, not that the user goes on at their own risk.

## Decision

Settings › Users gains a group above Accounts, which only the master sees:

```text
┌──────────────────────────────────────────────────────────────────────┐
│ Model Providers                                                      │
├──────────────────────────────────────────────────────────────────────┤
│ Providers                      [ Each User's Own | Shared ]          │
│ When shared, every user uses the API keys you set up.                │
└──────────────────────────────────────────────────────────────────────┘
```

- Each User's Own is the instance's mode until the master changes it:
  every user sets up their own providers. Shared gives every user the
  master's providers, which only the master sets up.
- Choosing the other asks first, saying what changes:

  ```text
  Share your providers with every user?
  Every user's model menu offers your providers instead of their own.
  Theirs stay saved and come back if you stop sharing. Subscriptions,
  yours included, aren't offered while providers are shared.
                                             [Cancel] [Share]

  Stop sharing your providers?
  Every user goes back to their own providers. Users who haven't set
  any up have no models until they do.
                                     [Cancel] [Stop Sharing]
  ```

- A change applies to every user at once: their pages show the providers
  and models of the new mode, and each session takes its entry from the new
  owner at its next request. A conversation whose model the new mode does
  not offer is one whose provider is gone, as after the provider's removal.
- A shared instance offers no subscription: Settings › Providers lists no
  Subscriptions group, a sign-in is refused, and the model menus and
  `demi providers` list no subscription model. The entries stay stored and
  come back with Each User's Own.
- `demi-server setup` asks no mode and takes no `--mode`, and the backend
  reads no `DEMI_INSTANCE_MODE`.

The warning before a second account or a rotation names no vendor and says
the risk is the user's; it still shows once:

```text
Use several accounts?
Using more than one subscription account, or rotating between accounts,
may violate the provider's Terms of Service. You do so at your own risk.
Demi won't ask again.
                                           [Cancel] [Continue]
```

The design:

- `server_settings` keeps the mode, `instance_mode TEXT NOT NULL CHECK
  (instance_mode IN ('shared', 'isolated'))`, and its upgrade channel
  becomes nullable, since a row may now hold a mode with no channel
  chosen. No row means isolated.
- The vault holds the mode, which it reads from the control store at start
  and writes after the store (`crates/backend-providers/src/vault/entries.rs`);
  the usage view, `GET /settings` and the product state read it there, in
  place of a mode fixed at start.
- `PUT /settings/instance-mode` takes `{ "mode": "shared" }` from the master
  only. The change marks every user's channel with a new part, `instance`,
  which carries the mode, and with their providers and models.
- The vault's `entries` and `entry` leave out a subscription entry on a
  shared instance, so every reader of the entries, the sessions included,
  meets none, and a sign-in is refused when it starts. Ordering the entries
  keeps the ones left out where they were.

What an older piece meets: a configuration file that still sets
`DEMI_INSTANCE_MODE` stops the backend at start, naming the variable, as
any variable no setting reads does; its line is removed before the
upgrade, and the master chooses the mode on the page.

## Spike

None.

## Prior art

- Open WebUI keeps its connections in Settings › Admin › Connections,
  which every user shares, and an administrator turns users' own
  connections, Direct Connections, on there at run time, or with
  `ENABLE_DIRECT_CONNECTIONS`
  ([Direct Connections](https://docs.openwebui.com/features/chat-conversations/direct-connections/)).
- Demi's own upgrade channel is kept in `server_settings` and chosen in
  Settings › Updates.

Demi departs from Open WebUI in having one mode, not two layers: a user
either has their own providers or the master's, and the setting has no
variable beside it.

## Alternatives

- **Keep the variable as the default the setting overrides,** as the
  upgrade channel's `DEMI_UPGRADE_CHANNEL` is. Two places would say the
  mode, and a server would show a mode the file no longer says.
- **Share API keys and subscriptions alike.** A personal subscription
  serving many users is what vendors' terms most clearly forbid.

## Open questions

None.
