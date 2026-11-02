---
status: proposed
---

# A permission covers a whole capability

Supersedes: 0140 (Sending to another conversation asks nothing), the part
that lets an agent send without a permission; 0204 (An agent saves a
credential from a pipe), the part that lets a save ask nothing: both now need
the permission of their capability, once per conversation.

## Problem

Alex lets a conversation use credentials and then answers card after card
for it. The agent reads the GitHub token: Alex allows that credential. It
reads the npm token: another card, for the same conversation and the same
kind of work. It moves to the build machine: a card for that device. It
attaches the Cloud: a card for the Cloud. It pairs a new computer: a card for
that pairing code. A conversation that a URL woke asks again before every
read of a credential, even one Alex allowed.

Each card asks about one object or one step of a capability Alex already
decided to give the conversation, so Alex learns to allow them without
reading, and the cards protect nothing.

## Decision

A conversation asks once per capability. Allowed, its agents use the whole
capability for the conversation's life, every object and every step of it,
with no other card.

| Capability | Category | Card asks to | Covers |
| --- | --- | --- | --- |
| Credentials | `credentials.use` | use your credentials | every `demi credentials` command: list, show, request, save, read, remove |
| Devices | `host.use` | use your devices | bringing in any of the user's devices, the Cloud included (attach, move), pairing a computer, port forwards |
| Conversations | `conversation.use` | use your other conversations | listing the user's conversations, reading another's transcript, sending to another |

- A category is named by the command group that declares it, as every
  category is: `credentials`, `host` and `conversation`.
- The card is the category card every other capability has: Deny, or Allow
  for This Conversation, with a description that says what the agents can
  then do, such as "Use Credentials lets the agents of this conversation
  list, read, save, replace and remove your saved credentials."
- These go: the allowance of a single credential and its card; the request
  to reach a single device and its card; the pass for a single pairing code;
  asking again before every read while a URL's run goes on; the categories
  `credentials.manage`, `host.forward` and `conversation.read`, which the
  three above replace.
- What a conversation reaches without asking stays: its primary Host, its
  project's device, the user's Cloud for a conversation there, and a device
  attached to it, which the user chose.
- The capabilities that already ask once stay as they are: providers,
  skills, subagent profiles and the conversation's workflow.

### Design

- `backend-database`: `may_reach(conversation, device)` answers yes for the
  primary Host, the project's device, the Cloud of a Cloud conversation, an
  attached device, or any device of the user once the conversation holds
  `host.use`. `reachable_hosts`, `pairing_passes` and
  `credential_allowances` go, and with them the `allowance` kind and the
  `credential_id`, `host_device_id` and `pairing_code` columns of
  `permission_requests`. `read_credential` returns the value or the
  category's refusal.
- The control schema's migration: a conversation with a row in
  `credential_allowances` or a grant of `credentials.manage` gets
  `credentials.use`; one with a row in `reachable_hosts` for a device other
  than its primary Host, or a grant of `host.forward`, gets `host.use`;
  `conversation.read`, and the `conversation.send` an earlier release left,
  become `conversation.use`. Requests of the kinds that go are dropped, and
  the others of a category that goes ask for the one that covers it.
- A receipt in a transcript keeps the words it was recorded with: an earlier
  decision that let the conversation reach a host or use a credential still
  says so.
- `backend-permissions`: `Needed::Host` and `AskedHost` go; a call that
  brings in a device the conversation may not reach, or pairs one, needs
  `host.use` as any category is needed. `Answering`, `PushAsk` and the
  page's request list lose the allowance.
- `web-api-protocol` `PermissionRequest` and `PushAsk` lose `Allowance` and
  the host of a permission request, and `ErrorCode` loses
  `credential_removed`; `web-ui` loses `AllowanceRequestCard.vue` and the host
  body of `PermissionRequestCard.vue`, and the gallery their specimens.
- The help of `demi credentials` says its commands need the permission once
  for the conversation; the other groups' help stays, since a card asks when
  a command needs it.

## Spike

None.

## Prior art

- iOS and Android ask once per capability (Contacts, Photos, Location) and
  never per item; iOS's limited photo library is the one exception, and it is
  the user's choice, not a card per photo.
- macOS's privacy settings grant an app Full Disk Access or Automation once,
  not per file or per command.
- Claude Code's permission rules allow a tool, or a command pattern, for the
  session.

## Alternatives

- Keeping per-object cards for the riskiest objects, such as pairing a new
  computer: it brings back the card the user allows without reading, and the
  description of the category card already says it.
- Keeping the per-read card while a URL's run goes on: the user decided the
  conversation's grants hold whoever woke it.

## Open questions

None.
