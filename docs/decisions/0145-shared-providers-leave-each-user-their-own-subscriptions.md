---
status: proposed
---

# Shared providers leave each user their own subscriptions

Supersedes: 0141 (Whether users share the master's providers is a setting),
the rule that a shared instance offers no subscription, and its alternative
that refused sharing subscriptions.

## Problem

Alex, the master, shares providers so that the team uses the API keys Alex
pays for. Bo, a user, has a Claude subscription of their own. While Alex
shares, Bo cannot use it: the instance offers no subscription at all, Bo's
sign-in is refused, and Bo's Settings has no Models page to sign in from. Bo
loses a subscription that serves nobody else and breaks no vendor's terms,
and Alex's keys pay for work Bo's subscription would have covered.

## Decision

While providers are shared, each user infers with the master's API keys and
with their own subscriptions. Nobody infers with another user's
subscription: Bo's model menu lists Alex's API keys and Bo's Claude, and
Alex's lists Alex's API keys and Alex's own subscriptions, without Bo's.

| | API keys | Subscriptions |
| --- | --- | --- |
| Each User's Own | the user's | the user's |
| Shared | the master's | the user's |
| Shared, `DEMI_SHARED_SUBSCRIPTIONS=on` | the master's | the master's |

- A user's own API keys stay hidden while providers are shared, stored as
  before, and come back when sharing stops.
- `DEMI_SHARED_SUBSCRIPTIONS=on` keeps what it does today: every user
  infers with the master's subscriptions, and no user signs in to one of
  their own.
- Every user manages the entries they infer with that they own: on a
  shared instance a user signs in to, removes, orders and reads the
  accounts, plan and usage of their own subscriptions, as on an isolated
  one, and changes none of the master's entries.

A user of a shared instance gets back Settings › Models, with their own
subscriptions. The API Keys group has no Add button and says whose keys
they use:

```text
┌─ Settings › Models ─────────────────────────────────────────────┐
│ SUBSCRIPTIONS               │                                   │
│  ◆ Claude                 ● │  Claude                           │
│  ◆ ChatGPT                  │  Accounts, plan and usage of      │
│                             │  the selected subscription, as    │
│ API KEYS                    │  on an isolated instance.         │
│  The master shares API keys │                                   │
│  with every user.           │                                   │
└─────────────────────────────┴───────────────────────────────────┘
```

Settings › Users says what the change leaves each user:

```text
Providers                      [ Each User's Own | Shared ]
When shared, every user uses the API keys you set up and their
own subscriptions.

Share your providers with every user?
Every user's model menu offers your providers instead of their own.
Theirs stay saved and come back if you stop sharing. Each user keeps
their own subscriptions, which nobody else can use.
                                             [Cancel] [Share]
```

With `DEMI_SHARED_SUBSCRIPTIONS=on` the description and the question read
as they do today, without the sentence on subscriptions.

The design:

- The vault answers whose entries a user infers with, by kind, in one
  place: `Vault::owners(user) -> Owners`, which names the owner of the API
  keys and of the subscriptions. `entries_for(user)` reads the
  subscriptions of the one, then the API keys of the other, each in its
  owner's order. It replaces `owner_for` followed by `entries`, which every
  reader of a user's entries called: the model catalog, the sync state,
  media, agent profiles, small requests, `GET /providers` and
  `demi providers`.
- `visible(user, id)` holds an entry whose owner is the owner of its kind.
- `configures(user)` goes. A user manages an entry they infer with when
  they own it, `Vault::manages(user, entry)`, which decides its changes and
  whether its accounts, plan and usage are read to them; and adds entries
  of a kind when they are its owner, `Owners::manages(kind)`, which decides
  adding an API key and a sign-in.
- A sign-in creates the entry for its starter, and is refused only when the
  starter infers with the master's subscriptions.
- A change of an entry marks the channels of every user who infers with
  it: everyone for the master's entries a shared instance shares, the
  owner alone for any other.
- `ProductState` and the instance part of the sync channel carry
  `manages: { apiKeys, subscriptions }`, whether each kind the user infers
  with is their own, in place of the page's own rule of mode and role.
  Settings › Models shows when either is; it lists the entries of the kinds
  the user manages, and says whose the API keys are when they are not the
  user's.

No stored data changes: a user's subscriptions on a shared instance are the
entries the vault already holds for them.

What an older piece meets: a page loaded before the upgrade goes on hiding
Settings › Models from a user of a shared instance until it reloads.

## Spike

None.

## Prior art

- Open WebUI shares the administrator's connections with every user and,
  when Direct Connections is on, lets each user add connections that only
  they use
  ([Direct Connections](https://docs.openwebui.com/features/chat-conversations/direct-connections/)).
- LibreChat lets an administrator define an endpoint whose key each user
  enters for themselves, `apiKey: "user_provided"`
  ([Custom Endpoints](https://www.librechat.ai/docs/configuration/librechat_yaml/object_structure/custom_endpoint)).

Both layer a user's own credentials on the administrator's. Demi layers
them by kind: the master's API keys and the user's own subscriptions, since
a subscription is a person's account, and a user's own API keys stay
hidden so the shared keys are the only ones the team pays for.

## Alternatives

- **Offer no subscription while shared,** as 0141 does. Every user of a
  shared instance loses a subscription that serves only them.
- **Offer each user their own API keys too.** The master shares to decide
  which keys the team uses; a user's own keys beside them would undo that.

0141 refused sharing subscriptions because one personal subscription would
serve many people. Here each subscription serves its own owner only, so
that reason does not apply.

## Open questions

None.
