# Web API: accounts and settings

The routes of a user's account and settings: accounts, preferences, subagents, instructions, provider configuration and subscription accounts. Conventions every route follows are in [Web API](web-api.md).

## Account API

`POST /api/setup` takes `{ nickname, email, password }`, the nickname as
`PATCH /api/auth/me` takes it, and `POST /api/auth/login` takes
`{ email, password }`; refused for too many failed sign-ins, it answers 429
with `Retry-After`, the seconds until it takes the address again.
The users routes are an administrator's: a user answers 403 `forbidden`.
`GET /api/users` returns `{ users }`, every account in the order they were
created. `POST /api/users` takes `{ email, role }` with role `admin`
or `user`, creates an account of a role the caller outranks with a first
password the server makes, and returns `{ user, password }` with 201; a role
the caller does not outrank answers 403 `forbidden`, an address an account
has 409 `email_taken`, and a session not verified in the last five minutes
403 `verification_required`. `POST /api/users/:id/password` makes a new
first password for a lower-ranked account, the account's earlier one
stopping, and answers `{ password }`, with the same verification.
`PATCH /api/users/:id` changes a lower-ranked account with one of
`{ role }`, `{ disabled }` or `{ cloudStorage }` and answers 204;
`cloudStorage`, `{ systemGb, homeGb }` with whole numbers from 1 to 1024,
may also change the caller's own account, and each account in `GET
/api/users` carries its own. A role the
caller does not outrank answers 403 `forbidden`, and disabling ends the
account's sessions and runner connections at once. `POST
/api/users/:id/sign-out` ends every session of a lower-ranked account and
answers 204. `POST /api/users/:id/sign-in-code` makes a sign-in code for a
lower-ranked account and answers `{ id, url, expiresInMs }` as
`POST /api/auth/handoff/code` does, with `to` the product's home; a session
not verified in the last five minutes answers 403 `verification_required`.
It ends that account's earlier sign-in code; `POST /api/auth/handoff` takes
it as it takes a handoff code, and the session it makes records the issuer. An id that names no account answers 404 `user_not_found` and an
account the caller does not outrank 403 `forbidden`, both before the body is
checked. A disabled account's sign-in answers 403 `account_disabled`, by any
way it signs in.
Setup creates the only master account and answers 404 `already_set_up` after
setup is complete. HTTP validation trims and lowercases addresses; storage
uniqueness and lookups are case-insensitive. A password being set, at setup,
as the `next` password, has 8 to 1024
characters; a password checked against the account's, at login, as the
`current` password or to start an email change, only has to be nonempty, so
a wrong one answers 401 `invalid_credentials` rather than a validation error.
User responses expose `id`, `email`, `nickname`, `role`, `createdAt`,
`disabled`, `choosesPassword`, true until the owner of an account that
Add User… made or an administrator's reset changed chooses its own
password, and `lastActiveAt`, the latest activity of any of its sessions,
never the password hash.

Passkeys ([User system](users.md#user-system),
[Authentication and ownership](../backend/backend.md#authentication-and-ownership)):

- `POST /api/auth/passkey/options` is public and returns `{ challengeId,
  options }`, WebAuthn's request options as JSON; with 1000 challenges
  waiting it answers 429 `too_many_attempts`. `POST /api/auth/passkey` takes
  `{ challengeId, credential }`, the browser's answer as JSON, and signs in
  as `POST /api/auth/login` does; an expired or used challenge answers 401
  `passkey_expired`, a passkey no account has 401 `unknown_passkey` with its
  `credentialId`, and a failed check 401 `invalid_credentials`.
- `GET /api/auth/passkeys` returns `{ passkeys, userHandle }`: each passkey
  `{ id, name, synced, createdAt, lastUsedAt }`, its `id` the credential id in
  base64url, as `unknown_passkey`'s `credentialId` and the browser name it,
  and `userHandle` the account's WebAuthn user handle, which the page passes
  to the browser's signal methods with the relying party's id. `POST /api/auth/passkeys/options` takes
  no body; a session not verified in the last five minutes answers 403
  `verification_required`, and the page verifies with `POST /api/auth/verify`
  first. It returns `{ challengeId, options }`, the creation options, which exclude the
  account's passkeys. `POST /api/auth/passkeys` takes `{ challengeId,
  credential }` and answers 201 `{ passkey }`; its name is the password
  manager's for the few managers most people use, whose AAGUIDs their
  vendors publish and the backend lists itself (Google Password Manager,
  Windows Hello, 1Password, Bitwarden and the like; Apple's passkeys carry
  none), and otherwise the browser and system of the page that added it,
  such as `Safari on macOS`.
  `PATCH /api/auth/passkeys/:id` renames one with `{ name }` (1–80
  characters after trimming), and `DELETE /api/auth/passkeys/:id` removes
  it, both 204; an id the account has no passkey of answers 404
  `passkey_not_found`. On a backend that offers no passkeys every passkey
  route answers 409 `passkeys_unavailable`. Adding answers as signing in
  does: an expired or used challenge 401 `passkey_expired`, a failed check
  or a passkey the account already has 401 `invalid_credentials`. The synchronization channel's `passkeys` part carries the
  list, so Settings on another page follows.
- The public `GET /api/setup` answer carries `passkeyRpId`, the relying
  party's id, the public URL's host name, or null when this backend offers
  no passkeys, as when that host is an IP address; the product state carries
  it too. The setup
  page adds the new account's first passkey with the password just entered,
  through the same routes, after `POST /api/setup` has signed it in.
Verification, handoff codes and sessions ([Authentication and ownership](../backend/backend.md#authentication-and-ownership)):

- `POST /api/auth/verify` takes `{ password }` and answers 204, or 401
  `invalid_credentials`; `POST /api/auth/verify/passkey/options` and
  `POST /api/auth/verify/passkey` verify with a passkey of the account as
  sign-in does, answering 204, or 401 `unknown_passkey` for another
  account's. Each marks the session verified now, as every sign-in does,
  by a password, a passkey, a handoff code or a sign-in code.
- `POST /api/auth/handoff/code` takes `{ to }`, a path of the product, and
  answers `{ id, url, expiresInMs }`, the page counting the time left from
  the answer's arrival, never comparing the server's clock with its own; a session not verified in the last five
  minutes answers 403 `verification_required`, a `to` that is not a path of
  the product 400 `invalid_body`. It ends the account's earlier code.
- `POST /api/auth/handoff` is public, takes `{ code }` and answers
  `{ to }`. With a session cookie of the code's account it keeps that
  session, records the code on it as `handoffId` and sets no cookie; with
  another account's, it ends that session and signs in as
  `POST /api/auth/login` does; with none, it signs in; a code that is unknown,
  expired or spent answers 401 `handoff_expired`.
- `GET /api/auth/sessions` returns `{ sessions }`, each `{ id, name,
  createdAt, lastActiveAt, current, handoffId, issuedBy }`, newest first,
  `issuedBy` the nickname of the administrator whose sign-in code made it;
  `DELETE /api/auth/sessions/:id` signs another session out and answers
  204, the current one answering 409 `current_session` (Sign Out ends it),
  an id the account has no session of 404 `session_not_found`. The
  synchronization channel's `sessions` part carries the list.
- `PATCH /api/auth/me` takes `{ nickname }` (1–80 characters after trimming).
- `PUT /api/auth/password` takes `{ current, next }` and checks the current password;
  an account whose `choosesPassword` is true sends `{ next }` alone. While
  it is true, every route of the account other than `GET /api/auth/me`,
  this one, `POST /api/auth/logout` and the passkey routes answers 403
  `password_choice_required`, no conversation socket of it opens, and its
  synchronization channel carries the account alone, whose change tells
  every page of it that the password was chosen.
- `POST /api/auth/email` takes `{ email, password }`, checks the current password,
  and sends a six-digit code to the new address. Its 202 response contains
  `{ challenge: { id, email, expiresAt } }`; the code is never returned.
- `POST /api/auth/email/confirm` takes `{ id, code }`. A successful response
  contains the updated user. Existing sessions keep their user identity; the
  next login uses the new address. Resending repeats the start request after
  its 60-second cooldown and replaces the previous code.

The backend issues the code and delivers it through its account mail sender.
Without a sender, the start answers 503 `mail_unavailable`; a delivery failure
answers 503 `mail_failed` and removes that challenge so the user can retry.
Codes expire after ten minutes, allow at most five wrong attempts, and can be
used only once. A password change invalidates the challenges issued before it.
Confirmation checks uniqueness, updates the email, and consumes the challenge
in one transaction. Tests use captured mail only. Setup/admin-created accounts
can sign in without a verification email; this flow proves the new address
when an existing account changes it. Registration and password recovery are
not provided.

## Push notifications

`GET /api/push/key` answers `{ publicKey }`, the backend's VAPID public key,
which the page subscribes with
([Notifications](turns.md#notifications)). `PUT /api/push/subscription`
takes `{ endpoint, keys: { p256dh, auth }, events: { turnFinishes,
turnFails, needsPermission, agentNotifies } }` and keeps it for the
session, one per session, replacing what the session had; the page sends it
when notifications are turned on and when a switch changes. `DELETE
/api/push/subscription` removes the session's and answers 204, when the
browser turns notifications off. Neither reaches another session's
subscription: an endpoint another live session holds answers 409
`push_endpoint_taken`.

A browser's subscription serves one session. The page unsubscribes the
browser at sign-out, and a session that finds the browser subscribed by
another session makes a new subscription before it sends one, so a push
queued for an account before it signed out is never shown after another
signs in: the browser drops what comes for an ended subscription. Signing
out, the session expiring or an administrator ending it removes the
session's subscription, and one the push service answers 404 or 410 for is
removed too. An endpoint is accepted only as an `https` URL whose host
resolves to public addresses, checked again as each send connects, and a
send follows no redirect, so a subscription cannot make the backend call
into its own network.

## User preferences

`GET /api/settings/preferences` returns `{ preferences: { appearance, shortcuts, lastModel?, lastWorkflow?, lastProjectHost?, language?, locale?, contextLimits?, sendWhileRunning?, resumeAtRetry?, resumeAfterRestart?, multiAccountWarned?, hiddenProviders?, hiddenModels?, smallModel?, smallModelSameProvider? } }`
for the signed-in user. These objects contain saved overrides; absent values use
the web app's defaults. `PATCH` accepts any subset of appearance fields
(`theme`, `tone`, `accent`, `fontSize`) and shortcut keys (`new`, `sidebar`,
`settings`). A null shortcut removes that override. `sendWhileRunning` is
`"steer"` or `"queue"`: what Enter does with a message while the agent works
([Steer or queue](turns.md#steer-or-queue)); absent means `"steer"`, and a
patch sets it. `resumeAtRetry` is whether a turn that failed with a known
time to try again, such as a usage limit that resets at 09:00, resumes by
itself at that time ([The unfinished turn](../agent/failures-and-recovery.md#the-unfinished-turn));
absent means `false`. `resumeAfterRestart` is whether a turn a
deployment, an upgrade or a restart interrupted goes on by itself once the
backend is back ([The unfinished turn](../agent/failures-and-recovery.md#the-unfinished-turn));
absent means `true`. `multiAccountWarned` is true once the user went on
past the dialog about using several subscription accounts
([Account rotation](../providers/providers.md#account-rotation)), which
then never shows again. Settings › Agent shows `sendWhileRunning`, `resumeAfterRestart` as *Resume after Demi
restarts* and `resumeAtRetry` as *Resume after a usage
limit resets*, and so does the error record of such a failure, as *Resume at
09:00* ([Recovering an unfinished turn](turns.md#recovering-an-unfinished-turn)). `smallModel` is the model settings Settings › Agent chose for small jobs, `{ providerId, modelId, thinkingEffort }`, absent for Automatic, and `smallModelSameProvider` whether small requests stay on the conversation's provider, absent means `false` ([Small model](../providers/models.md#small-model)). `hiddenProviders`, provider entry ids, and `hiddenModels`,
`{ providerId, modelId }` pairs, are the switches of Settings › Providers that
take an entry or a model out of the model menu without deleting it; a patch
replaces the list it names. They are the user's, not a browser's: every
device's menu, the profile editor's menu and `demi providers list` leave out
the same entries and models
([Managing profiles](../agent/subagents.md#managing-profiles)). A conversation
or a profile already on a hidden model keeps it; the menus no longer offer
it, and a profile change through `demi agent profiles` refuses it. **What a new conversation starts with** is remembered per project, and once
for all the conversations outside a project: the model settings and the
workflow last chosen there. For example, the user picks Opus and Parallel
Development in a conversation of the project *demi*, and Sonnet in a
conversation outside any project: the next conversation in *demi* starts
with Opus and Parallel Development, the next one outside a project with
Sonnet and No Subagents, and one in a project never used before with what
the conversations outside a project start with; a project that remembers
only one of the two takes the other from there. A project keeps
its choice on its workspace (`lastModel`, `lastWorkflow`), deleted with it;
the conversations outside a project keep theirs in the preferences
(`lastModel`, `lastWorkflow`). `lastModel` holds model settings,
`{ providerId, modelId, thinkingEffort, serviceTierId }`
([A conversation's model settings](../providers/models.md#a-conversations-model-settings)),
and `lastWorkflow` is a workflow choice as a conversation holds it,
`{ type: "default" }`, `{ type: "none" }` for No Subagents or
`{ type: "workflow", id }`; absent means No Subagents, as does a workflow the directory no longer
has. Every explicit choice of a model, an effort, a tier or a workflow saves
it where the conversation belongs, in an unsent draft or in an existing
conversation, and a conversation moved into or out of a project saves
nothing. The page saves a project's choice with
`PATCH /workspaces/:id { lastModel?, lastWorkflow? }` and the outside one
with the preferences' `PATCH`; both reach the user's other pages as a
rename does. A new conversation starts with it, and
its first send writes it to the new conversation's record, an effort the
model no longer offers replaced by the model's first effort and a tier it no
longer offers by the vendor's default. Its effort is null only for a model
that lists no efforts. From then on the
conversation has its own settings, which a later change of the preference does
not touch. `lastProjectHost` stores where New Project starts,
`{ kind: "cloud" | "device", deviceId? }`: the kind and the device the user
last chose in it ([Conversations and projects](conversations.md#conversations-and-projects)).
A patch replaces it whole; a `deviceId` the user no longer has is kept and
the dialog chooses no device for it. `language` is `system`, `en`,
`zh-Hans`, `zh-Hant`, `ja` or `ko`, absent meaning `system` ([Localization](localization.md#choosing-the-language)). `locale` stores the time
zone and languages the user's browser last reported, as
`{ timeZone, languages, language?, colorScheme? }` with an IANA zone name, BCP 47 tags in
preference order, the language the page resolved for *System*
([Localization](localization.md#choosing-the-language)) and the page's `light` or `dark` scheme. A page sends it
when its own facts may have changed: when it starts, when its browser's
languages or its theme change, and when the user comes back to it (focus or
the page becoming visible), so the page the user uses last is the one
stored. It sends only what differs from what is stored, counting a value it
sent and the backend answered in its canonical form as the same, and never
because the stored value changed: a stored value another page reported, or
the backend's canonical spelling of its own report, is no reason to report
again, so two pages with different languages or schemes, or a browser whose
tags the backend respells, do not report back and forth. A time zone the backend
does not know, a malformed language tag, or more than 16 languages is
rejected. The backend keeps the time zone in its IANA spelling, each tag in
its canonical form and each language once, in the reported order: the report
`{ timeZone: "asia/shanghai", languages: ["zh-cn", "EN", "zh-CN", "iw"] }` is
kept as `{ timeZone: "Asia/Shanghai", languages: ["zh-CN", "en", "he"] }`,
as `Intl.getCanonicalLocales` in the user's browser would write the tags.
Commands receive it in their
[command context](../execution/native-runtime.md#command-context), and the
[conversation browser](../browser/browser.md#native-driver) starts with it.
`contextLimits` holds the user's
[context limits](../providers/models.md#context-limit), by provider entry id
and then model id, each in tokens, such as
`{ "<entry id>": { "claude-opus-5-5": 300000 } }`; a model it does not name
uses its full window. A patch changes one model's limit with
`contextLimit: { providerId, modelId, tokens }`: 500000, 300000 or 200000
stores that limit, null removes it, and any other number is refused. The
backend does not look the model up: a limit its window does not offer has no
effect. The backend reads, merges and writes in one transaction, preserving
concurrent changes to other fields. A page sends a change of a switch at once,
and a change that comes in bursts, such as a slider's, after a short pause;
when the page is hidden, reloaded or closed, it sends what is still waiting
at once, with `keepalive`, so the user's last choice is the one stored. Preferences persist across restarts and are
separate for every user in both instance modes. The web app reads and
writes these overrides through its preference state adapter.

## Subagents

The caller's subagent settings, which the settings page's Subagents section
shows, are part of the product state, as `subagents`:
`{ enabled, maxLive, maxDepth, profiles }`. `enabled` is the
[Subagent switch](../agent/subagents.md#profiles), true for a user who never
turned it off. `maxLive` and `maxDepth` are the
[limits](../agent/subagents.md#limits), Maximum subagents and Maximum depth,
each `{ limit, unlimited }`: the whole number from 1 the user last set, kept
while `unlimited` is true so that Limited brings it back; `{ limit: 10,
unlimited: false }` and `{ limit: 1, unlimited: false }` for a user who never
set them. `profiles` holds the caller's
[subagent profiles](../agent/subagents.md#profiles), in name order, each
`{ id, name, description, model, instructions, canSpawn, enabled }`:

| Field | Value |
|---|---|
| `id` | The backend's id for the profile, which its name can change under |
| `name`, `description` | The `--profile` value and when the agent should use the profile |
| `model` | null for the parent's model, or model settings `{ providerId, modelId, thinkingEffort, serviceTierId }`, as a conversation's ([Sidebar mutations and read state](web-api-conversations.md#sidebar-mutations-and-read-state)); `thinkingEffort` is always an effort the model lists or `disabled`, never null |
| `instructions` | null for none, or the text added to the child's system prompt after the product's |
| `canSpawn` | Whether the profile's children may spawn children of their own |
| `enabled` | Whether agents may use the profile |

`PUT /api/subagents` takes any subset of `{ enabled, maxLive, maxDepth }`, a
limit as a whole `{ limit, unlimited }`, sets those for the caller and
answers 204; a `limit` that is not a whole number from 1 answers 400
`invalid_body`. The dialog that confirms Unlimited is the page's.

`POST /api/subagents/profiles` takes `{ name, description, model,
contextLimit, instructions, canSpawn }`, where `contextLimit` is null, the
user's limit, or the tokens of the profile's own choice, the full window
included (400 `invalid_body` without a model of its own or for a choice the
model does not offer), creates the profile enabled, and answers 201 with
`{ profile }`. `PATCH /api/subagents/profiles/:id` takes any subset of those
fields and `enabled`, and answers `{ profile }`; `model` and `instructions`
are whole values, and null returns either to the parent's.
`DELETE /api/subagents/profiles/:id` answers 204. An id that names none of
the caller's profiles answers 404 `profile_not_found`. The backend checks a
body before it writes:

- A name that breaks the [name rule](../agent/subagents.md#profiles),
  `default` included, a description that is blank, longer than 500
  characters or holds a line break, and instructions that are blank or longer
  than 65,536 characters answer 400 `invalid_body`.
- A name another of the caller's profiles has answers 409 `profile_exists`.
- A model is checked against the entry's catalog as a conversation's model
  settings are, with the same answers: 404 `provider_not_found` for an entry
  outside the caller's scope, 404 `model_not_found`, and 409
  `setting_unavailable` for an effort or a tier the model does not offer. A
  body whose model names no effort gets the first effort the model lists
  ([A conversation's model settings](../providers/models.md#a-conversations-model-settings)).
  Turning a profile on or off checks nothing else, so an unavailable profile
  can be disabled and enabled as it is.

Each change, to the switch or to a profile, reaches every page of the user as
the `subagents` message of the [synchronization channel](web-api-sync.md#page-synchronization),
and the next spawn of every conversation, an open one included
([Profiles](../agent/subagents.md#profiles)). The settings are the user's in
both instance modes; on a shared instance a profile's model names one of the
master's entries, as every model a user picks does.

## Instructions

The caller's [personal instructions](../agent/instructions.md#personal-instructions),
which the settings page's Instructions section shows, are part of the
product state, as `instructions`: their text, empty for a user who has none.

`PUT /api/instructions { text }` replaces them and answers 204; an empty text,
or one of only white space, removes them. A text longer than 65,536
characters answers 400 `invalid_body`. Each change reaches every page of the
user as the `instructions` message of the
[synchronization channel](web-api-sync.md#page-synchronization), and the next request of
every node of the user's conversations.

## Model configuration and provider inspection

`POST /api/providers` takes
`{ source: "vendor", vendorId, label?, apiKey?, models? }`,
`{ source: "family", providerType, wireApi?, label?, apiKey?, models? }` or
`{ source: "compatible", providerType?, wireApi?, label?, apiKey?, baseUrl?, models? }`:
a vendor entry of a models.dev vendor, a family's own vendor's entry, such as
Anthropic's, and a compatible entry, Anthropic Messages unless
`providerType` and `wireApi` say otherwise
([Endpoints](../providers/providers.md#endpoints)). Everything but the kind
may come later, and the entry is
[incomplete](../providers/models.md#a-provider-is-in-the-list-from-the-start)
until it has what it needs; `label` defaults to the vendor's or family's
name, or *Compatible API*, with the first free *(2)*, *(3)* when the user
has an entry of that name.
The first derives the family, its wire and its endpoint from the vendor
catalog, `GET /api/providers/catalog`, which lists the
[vendors from models.dev](../providers/providers.md#vendors-from-modelsdev);
the second names the family explicitly. `wireApi` is
`responses` or `chat-completions`, and only a family that speaks both takes
it. A label has 1 to 80 characters after trimming, a key is one line of text,
and `baseUrl` is an `http` or `https` URL. PATCH accepts label and, for
API-key entries, key (null removes it) and model list, and a compatible
entry's endpoint (null removes it, which leaves the entry incomplete),
`providerType` and `wireApi`; a subscription entry takes only a new label
(400 `subscription_only`), and an endpoint or a protocol set on a vendor
entry answers 400 `endpoint_kind`, since an entry stays the kind it was
added as. A model list whose ids repeat answers 400 `duplicate_model`. An entry's
read model carries `endpoint`, the base URL its requests go to, whichever
kind it is. The Test endpoint deliberately makes a real inference
request. A change of an entry, its accounts included, holds the entry while it
runs; another change meanwhile answers 409 `provider_busy`.

API providers accept `models` as a complete manual list of 0–1000 entries; an empty list leaves the entry [incomplete](../providers/models.md#a-provider-is-in-the-list-from-the-start). Each entry supplies
`id`, `displayName`, `contextWindow`, positive or null when unknown, nullable `outputLimit`,
`thinkingEfforts`, nullable `acceptedExtensions` (without dots) and nullable
`fastTier`. IDs must be unique and output limits cannot exceed context length.
`models: null` on PATCH explicitly returns to the live catalog; refreshing never
clears saved models. A subscription entry's catalog comes from its provider.

How a manual list's models become a conversation's model selection, and how
an edit of the list reaches a running conversation, is defined in
[Request parameters](../providers/models.md#request-parameters). Changing a
catalog never resends a failed message.

`GET /api/models` is an account-wide catalog, independent of any conversation.
Each provider carries its models, `sourceFetchedAt`, `stale` and `warnings`,
its authentication and runtime health, the `availability` the backend derives
from that health, and `cliPackage`: the command package that installs the
CLI its requests start on the Claude Code machine, such as `demi.claude-code`,
or null when its requests are HTTP; the page knows by it that the provider
runs a process. The entries of the product state carry the same `cliPackage`. `availability` is
`{ type: "unavailable", reason: "setup", message, missing }` while the entry
is incomplete, `missing` naming what it lacks (`apiKey`, `endpoint`, `models`,
`account`)
([A provider is in the list from the start](../providers/models.md#a-provider-is-in-the-list-from-the-start)),
`{ type: "unavailable", reason: "authentication", message }` while the
credential is refused or a subscription's account expired, `{ type: "unavailable", reason: "runtime",
message }` with the runtime's message while the provider cannot run, and
`{ type: "available" }` otherwise, unknown health included. Each model carries
the `selection` the backend built from it. The first effort of a model's
`supportedThinkingEfforts` is the one a conversation's model settings hold on
the model when a change names none
([A conversation's model settings](../providers/models.md#a-conversations-model-settings)).
One provider's catalog failure does not remove the other providers or saved
models. A static catalog, or one never fetched, reports the Unix epoch as
`sourceFetchedAt`. `refresh=true` waits for
a shared forced refresh; how catalogs are cached and refreshed is defined in
[Catalog cache](../providers/models.md#catalog-cache). The route does not wake
Cloud or execute a model. The web app combines each provider's health with the
state of the conversation's target and, for a provider whose transport is a
process, of the user's Cloud. Inference admission checks the actual target.
Unknown auth/runtime status stays explicit; catalog availability is not a
successful inference test. Beside the providers, `smallModel` is the selection Automatic resolves
to for the [small model](../providers/models.md#small-model), as for a
conversation on the first entry with the user's hidden entries and models
left out, or null when Only the Conversation's Provider is on or no tier
matches; Settings › Agent shows it beside *Automatic*.

`GET /api/models/search?q=<query>&entry=<id>` answers `{ results }`, the
models.dev models the query matches, filled from `entry`'s vendor where it
lists them ([Adding a model from models.dev](../providers/models.md#adding-a-model-from-modelsdev)):
each `{ ref, displayName, id, vendor, contextWindow, outputLimit,
thinkingEfforts, acceptedExtensions, releaseDate }`, `ref` being
`<vendor>/<model id>`. It reads the backend's kept copy and asks models.dev
only when there is none.

`GET /api/providers/:id/status` returns auth/runtime state, account metadata,
active account, capabilities and quota. An entry whose provider cannot be built
or read answers 503 `provider_status_failed` with the reason, the message its
`failed` details carry in the sync state. Each account carries its own `quota`,
the last real snapshot kept for it; the top-level `quota` is the active
account's. `quotaCapability` is `{ type: "none" }` for a family without quota,
or `{ type: "supported", probe }` with the probe's cost, `free` or
`inference`, or null for a family that only observes. No query returns
key/token material or raw vendor quota envelopes.
`POST /api/providers/:id/quota` takes `{ credentialId? }`, or no body, and
refreshes that account's free quota probe (the active account's without one);
cancelling the request stops the probe. An unknown account is
`account_not_found`. A provider requiring inference for a probe returns
`quota_requires_inference`, and a probe the vendor fails answers 503
`quota_unavailable`, with the vendor's reason; a family that cannot probe answers the kept snapshot, and
no data returns null, not a fabricated percentage
([Vendor quota](../providers/usage-and-quota.md#vendor-quota)). Reading status
never invokes inference.
`POST /api/providers/:id/test` takes `{ modelId, credentialId? }` and tests
with that account, in use or not; an unknown account is `account_not_found`.
The test sends one request to the model and answers `{ type: "passed", model }`
when the provider answers, or `{ type: "failed", message, model? }` with the
provider's own reason: a test that ran and failed is a result, not an error. A
provider whose transport is a process is tested on the Claude Code machine,
where its requests run too
([Where it runs](../providers/claude-code.md#where-it-runs)); the test wakes
it.

The CLI of an entry whose provider runs a process is read and managed under
`/api/providers/:id/cli`; for any other entry these routes answer 404
`provider_not_found`:

| Route | Meaning |
|---|---|
| `GET …/cli?refresh=` | `{ newest, install }`: the vendor's newest version (`{ type: "read", version }`, or `{ type: "unreadable", message }` when the vendor cannot be read; `refresh=true` asks it at once), and the Claude Code machine's last install since the backend started (`{ state: "installing" }`, `{ state: "installed", path }`, `{ state: "failed", message }`, or null), whose `path` names the installed version. It wakes nothing. |
| `POST …/cli/install` | Starts the install on the Claude Code machine again, unless one is under way; 202 `{ install }` with its state. |

Adding an account to such an entry starts the same install on the Claude
Code machine; its failure is this state and never the failure of adding the
account.

On a shared instance every user reads provider state, but only the master sees
accounts, plan and usage: for everyone else `accounts` is empty, `active` names
no account, `quota` is null and an authenticated `auth` carries no account
label, in the status, in the account list and in the product state alike.
Configuring, testing and refreshing usage are the master's, and anyone else is
refused with 403 `forbidden` ([Scope](../providers/providers.md#scope)).

## Subscription accounts

Every subscription account comes from a sign-in, below.
`GET /api/providers/:id/accounts` lists public
account metadata and the active account as `{ accounts, active }`. An account
whose sign-in the vendor refused carries `signedOut: true`
([Accounts and sign-in](../providers/claude-code.md#accounts-and-sign-in));
its row says *Sign In Again* and offers that action, which starts
`…/accounts/login` and replaces the account's sign-in when it names the same
account.
`PUT …/accounts/active` takes `{ credentialId }` and answers `{ active }`.
`PUT …/accounts/order` takes `{ credentialIds }`, every account of the entry
once, in the order In Order uses and the list shows. `PATCH
/api/providers/:id` takes `rotation`, `manual`, `in_order` or `balanced`
([Account rotation](../providers/providers.md#account-rotation)); an entry
with fewer than two accounts keeps `manual` (400 `rotation_needs_accounts`).
`DELETE …/accounts/:credentialId` removes any account, the active one
included, and answers `{ active }`, the account now active or null when the
entry has none left
([Login and publication](../providers/providers.md#login-and-publication)). An entry that does not take accounts this way answers 400
`accounts_unsupported`.

Codex, Grok Build and Claude Code use `POST /api/providers/subscription-login { providerType, label? }`
for the first account and
`POST /api/providers/:id/accounts/login` for another account on an existing
provider. A start returns 202 with `{ login: { id, status: "pending" } }`; a
family without a device login answers 400 `no_login_flow`. The login's state
reaches every page of its starter on the sync channel's `logins` message
([Page synchronization](web-api-sync.md#page-synchronization)), and
`GET /api/providers/subscription-login/:id` answers the same `{ login }`:
`{ status: "pending", stage, verificationUrl, userCode, expiresAt }`, where
`stage` says what the login waits for (`preparing`, then `waiting_for_user`
once it has something to show; Claude Code adds `waiting_for_machine`,
`installing` and `starting` before it,
[Accounts and sign-in](../providers/claude-code.md#accounts-and-sign-in)), the
`verificationUrl` and `userCode`
null until the vendor names them and `expiresAt` the moment the login ends
unless the user finishes it, null while it is `preparing` and set, with its
own announcement, once the login knows its kind, and for Claude Code `needsCode: true` and no
`userCode`: `verificationUrl` is the link the CLI printed, the user signs in
there and pastes the code the page shows, which
`POST /api/providers/subscription-login/:id/code { code }` hands to the CLI's
login as its next code and answers `{ code }`, the number the login gave
it, counting from one (409 `login_not_waiting` when it waits for none);
`codesTaken` counts the codes the login took. A code the CLI refuses leaves
the login pending with `codeError`, the CLI's words, until the user pastes
another; `codeError` is always the refusal of code number `codesTaken`, so a
refusal of an earlier code that arrives after a later one was taken is
dropped. A page shows a refusal only of the code its own request was
answered for, and *Signing in…* until that answer comes, so of two tabs
that paste a code each, neither shows the other's refusal as its own. The login then continues as a device login does, `{ status: "completed", providerId,
credentialId }` with the account the login added, `{ status: "cancelled" }`
once the user cancelled it, or `{ status: "failed", message, requestId }`,
where `requestId` is the id of the request that started the login, under
which the server log holds its whole course. A page learns
that a login completed no sooner than it learns the account: the vault
publishes the entry before it ends the login, the channel reads the parts of
one message in the order of the table below, providers before logins, and
reads each part only after the change that marked it, and the snapshot reads
the logins before the providers. A page therefore shows a completed login as
done the moment it learns of it, and an account removed later does not undo
that.
Cancel with
DELETE at the same path. A login is its starter's to read and cancel; another
id answers 404 `login_not_found`. Terminal results are retained for ten
minutes. A start on an entry that a login of the same user reserves answers
409 `provider_busy` with `loginId`, that login's id, so the page opens it
rather than a second one; a reservation of another kind names none. Login lifetime, credential publication, and account
selection follow [Providers](../providers/providers.md#credential-vault).

Provider account mutations obey the same shared-master/isolated-owner rule as
configuration. A switch takes effect at the next request, while an
already-running request finishes with its original runtime. An entry without
an account is refused before inference; the backend never falls back to a
vendor login of the machine it runs on.
