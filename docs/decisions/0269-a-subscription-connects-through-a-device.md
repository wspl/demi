---
status: proposed
---

# A subscription connects through a device

Extends 0199 (A provider's requests go out through a device) from API-key
entries to the ChatGPT and Grok subscriptions.

## Problem

Alex's Demi server is in a data center, and Alex wants their ChatGPT
subscription's requests to leave from the MacBook at home, as an API-key
entry's can since 0199. The ChatGPT page in Settings has no Connect Through
row, and the backend refuses a device for any entry that is not an API
key's, so every turn, token refresh and sign-in of the subscription goes out
from the server.

## Decision

The page of a subscription whose requests the backend makes, ChatGPT and
Grok, has Connect Through, the same row an API-key entry has, below its
accounts and their Rotation, which belongs to the list and follows it. It
is the provider's, not an account's: Server by default, and
every request of every account, its sign-in included, goes out from the
device chosen.

```text
ChatGPT
  alex@example.com                            Active
  sam@example.com
  Rotation          [In Order v]
  ----------------------------------------------------------
  Connect Through   [alex-mbp v]   Server / alex-mbp / build-01 (Offline)
                    Requests go out from alex-mbp, which reaches the
                    endpoint on its own network.
```

- Everything 0199 says of the choice holds: an offline device fails the
  request at once, a removed one fails it until another is chosen,
  `demi providers` sets it, and an administrator's shared subscription goes
  out through the administrator's device for every user of it.
- A sign-in of the provider goes out through its device too, so a sign-in
  through an offline device fails at once with "alex-mbp is offline".
- A subscription nobody has signed in to yet is a placeholder on the page,
  with no entry behind it. Choosing a device there creates the entry, with
  no account, as an entry whose last account was removed is, and Add Account
  then signs in to that entry; Add Account on a placeholder still creates
  the entry when the sign-in completes.
- Claude Code's page has no Connect Through: its requests are its CLI's,
  which runs on another machine than the backend, and where that CLI runs
  is a decision of its own.

The design to build from:

- A family says whether the backend makes all of its requests with the
  entry's client: `ProviderFamily::connects_through(&self) -> bool`, true by
  default, false for Claude Code and for an entry another server lends,
  whose runs go over a socket to the lender. Each entry carries it as
  `connectsThrough`, and the catalog's `SubscriptionFamily` too, for a
  subscription with no entry yet; the page reads it to show the row. It is
  not `cli_package`, a provider's: a borrowed entry has none and still
  cannot connect through a device.
- `through_device` in a provider patch is taken for any entry whose family
  connects through; another refuses it with `ConnectsDirectly`, which
  replaces the `SubscriptionOnly` this check answered.
- `POST /providers` takes `CreateProvider::Subscription { provider_type,
  through_device }`, which creates a subscription entry with no account,
  named after its family; the page sends it when a device is chosen on a
  placeholder. A second entry of the same family is refused as a sign-in's
  is, with `provider_exists`.
- Requests already follow the entry's device through
  `ProviderRegistry::http_for`, whatever the entry's credential, so quota,
  token refresh, model lists and media need no change. Codex's turns open a
  WebSocket first, which a device does not carry: an entry that connects
  through a device sends them as server-sent events alone
  (`ProviderHttp::goes_through_device`).
- Creating the entry holds the family's new entry as a sign-in on the
  placeholder does, so the two cannot both make it: the second is refused
  as busy, and the sign-in keeps its account.
- An older page shows no row and sends no device. The providers table drops
  the CHECK that kept the device to API-key entries; no row changes.

## Spike

None.

## Prior art

- CLIProxyAPI, which serves Codex and Claude subscriptions signed in by
  OAuth, sends their requests through one `proxy-url` for the whole server
  ([configuration](https://pkg.go.dev/github.com/router-for-me/CLIProxyAPI/v6@v6.2.23)).
  Demi's choice is each provider's, as 0199's is.

## Alternatives

- A device for each account: two accounts of one subscription are the same
  service, and a provider's one choice is what 0199 made.
- Keeping a placeholder's choice on the page until a sign-in creates the
  entry: the choice would be lost with the page, and the entry would be
  created by something other than its first edit.

## Open questions

None.
