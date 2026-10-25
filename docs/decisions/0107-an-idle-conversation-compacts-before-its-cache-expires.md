---
status: proposed
---

# An idle conversation compacts before its prompt cache expires

## Problem

Alex works with Claude Opus in a conversation whose context has grown to
400,000 tokens, then leaves for lunch. Demi asks Anthropic to keep the
conversation's prompt cache for an hour. Alex comes back after 90 minutes and
sends one line:

* The cache expired 30 minutes earlier, so the request writes all 400,000
  tokens to the cache again, at twice the input price: about 800,000 tokens'
  worth of input for one short message.
* The reply also takes longer to start, because the whole history is
  processed again.

A summary written before the cache expired would have read the history from
the cache, at a twentieth of the input price on Opus, and the next message
would have sent the summary instead of the history.

Two things keep Demi from doing that today:

* It compacts only when the context nears the model's window, or when the
  user asks.
* It does not know how long a conversation's cache lasts. Each provider
  sends what it hardcodes, if anything: Anthropic's requests ask for an hour,
  OpenAI's and Codex's leave the vendor's default, and Claude Code chooses
  for itself.

## Decision

### What the user sees

**Settings › Agent** gets a Context group under Messages:

```text
Context
┌──────────────────────────────────────────────────────────────────────┐
│ Compact idle conversations                                      [ ]  │
│ Shortly before a long conversation's prompt cache expires,           │
│ summarizes it so your next message doesn't resend all of it.         │
├──────────────────────────────────────────────────────────────────────┤
│ Minimum cache lifetime                       [30 Minutes | 1 Hour ]  │
│ Only conversations whose provider keeps the cache at least this      │
│ long are compacted.                                                  │
└──────────────────────────────────────────────────────────────────────┘
```

* The switch is off by default, and the lifetime is 1 Hour by default:
  a summary replaces history the user may have wanted kept, so the user
  opts in.
* While the switch is off, the lifetime row is dimmed and takes no input,
  and it keeps its value.

An API-key entry whose protocol has a parameter for the cache's lifetime
gets a **Prompt cache lifetime** row in its settings card, under API key. A
subscription gets none: its vendor's own client chooses the lifetime, and
Demi does not override it.

```text
┌──────────────────────────────────────────────────────────────────────┐
│ Prompt cache lifetime                          [Custom ⌄]  [ 2h   ]  │
│ How long the provider keeps a conversation cached. Unspecified       │
│ sends no lifetime.                                                   │
└──────────────────────────────────────────────────────────────────────┘
```

The menu lists Unspecified, then the presets the protocol knows, then
Custom…. Custom shows a field for any lifetime written
as whole minutes or hours, such as `2h` or `90m`. Demi sends it as written,
in the protocol's own field.

| Provider                     | Field Demi sends             | Presets              | Default     |
| ---------------------------- | ---------------------------- | -------------------- | ----------- |
| Anthropic Messages           | `cache_control.ttl`          | 5 minutes, 1 hour    | 1 hour      |
| OpenAI Responses             | `prompt_cache_options.ttl`   | 30 minutes, 12 hours | Unspecified |
| OpenAI Chat Completions      | `prompt_cache_options.ttl`   | 5 minutes, 1 hour    | Unspecified |
| Gemini and every subscription | none                        | no row               |             |

* An Anthropic entry, its own or a compatible one, keeps the hour Demi
  sends today.
* A compatible entry offers the presets of its protocol. Kimi takes
  `prompt_cache_options.ttl` on Chat Completions with 5 minutes and 1 hour.
  When its protocol changes, it keeps its lifetime only if the new protocol
  lists it as a preset, and takes the new protocol's default otherwise, so a
  lifetime typed for one endpoint is never sent to another unasked.
* A Custom lifetime the field cannot take, such as `2d`, stays in the field
  with what to type under the row, and changes nothing.
* Codex could not have a row anyway: its backend refuses
  `prompt_cache_options` for every model it serves (see [Spike](#spike)).

When a conversation is idle, the transcript shows the compaction's usual
divider, which says so:

```text
──────────── Context compacted while idle to ~18K tokens ────────────
```

### When a conversation compacts

The root agent of a conversation, never a subagent, compacts while idle when
all of these hold:

1. **The switch is on.**
2. **The model's window is large.** It is at least 300,000 tokens. On a
   smaller window, compaction near the window comes first.
3. **The context is large.** The last answered request carried at least
   200,000 tokens.
4. **The lifetime is known and long enough.** The last answered request's
   cache lifetime is known, and it is at least the setting's minimum.
5. **The conversation is idle.** No turn runs. A command that runs on a Host
   without a turn does not count, because it reads no cache.
6. **The cache is about to expire.** Nine tenths of the lifetime have passed
   since that request was sent: minute 27 of 30 minutes, minute 54 of an
   hour, hour 10.8 of 12 hours.

A request's cache lifetime is known in these cases:

* **An API-key entry** knows it only when the entry specifies one, as a
  preset or a custom value. Unspecified leaves it unknown, however long the
  vendor's default is.
* **Codex** knows OpenAI's documented default: at least 30 minutes for
  GPT-5.6 and later.
* **Claude Code** reads which lifetime the reply's usage reports writing to:
  an hour when all of the write is `ephemeral_1h_input_tokens`, else five
  minutes. That is an hour within the plan's usage, and five minutes beyond
  it. A reply that wrote nothing keeps the lifetime of the last write.
* **Grok Build** leaves it unknown. xAI documents no lifetime.

The pass is the one the user's Compact runs, with its summary request and
its kept messages:

* A message sent during the pass waits for it, as it waits for the user's
  Compact.
* The pass needs no Host, so it starts no Cloud.
* A pass that fails, for a usage limit or an error, leaves the conversation
  as it was and writes nothing to the transcript. The log says why. It is
  not tried again, because the cache has expired by the time it could be.

### How it works

```text
provider ── answered request + cache lifetime ──► agent session (root)
                                                       │ cache_due
                                                       ▼
user shard: idle_compactions row (control store) ◄── due time, lifetime
     │ the conversation's timer, sleeps until its due time
     ▼
opens the tree if needed ─► root's `compact` action, trigger Idle
```

* **The provider entry.** It holds the lifetime, and each provider crate
  declares the presets of its protocol:

  ```rust
  /// A prompt cache lifetime as a request sends it: whole minutes or hours,
  /// such as `5m`, `1h` or `90m`.
  pub struct CacheTtl(String);
  ```

  The `providers` table gains `cache_ttl TEXT`, plain like the label, since
  it is no secret. None is Unspecified. `ProviderFamily::cache_ttl_presets`
  answers the presets of a wire, none for a family without the parameter
  and for every subscription. `ProviderDto` carries `cache_ttl` and
  `cache_ttl_presets`, null for an entry without the row. A migration gives
  every existing Anthropic-protocol entry `1h`, the value it sends today.

* **The request.** Each provider's configuration carries the entry's
  `cache_ttl`, and the provider sends it in its field on the requests that
  use the session's cache. It reports the lifetime it knows by the rules
  above as a `CacheLifetime` event before the request's response.

* **The session.** When the root's request is answered and conditions 2 to 4
  can hold, the session reports the time the request was sent plus nine
  tenths of the lifetime, with the lifetime. It reports none otherwise, and
  none when a turn starts or a compaction ends.

* **The table.** The shard writes the report to `idle_compactions`, one
  row per conversation (`conversation_id`, `due_at`, `lifetime_ms`) in the
  control store, deleted with its conversation. A backend that restarts
  reads the rows, without opening any conversation's database. The
  transcript holds no copy.

* **The shard.** Each conversation with a row has a timer, as a failed
  turn's retry time has. At the due time it takes the row, and does nothing
  when the cache has already expired, as after a backend that was down
  meanwhile. It reads the user's settings, so a change applies to the
  conversations that wait. It then opens the tree if it is closed and asks
  the root for a `compact` action with the trigger `Idle`. The root admits
  it only while no turn runs. The backend's start opens the shard of each
  user who has a row, which arms the timers again.

* **The divider.** The compaction marker gains `idle: bool`, false by
  default, which turns *Context compacted to ~N tokens* into *Context
  compacted while idle to ~N tokens*.

* **The settings.** `Preferences` gains `compact_idle: Option<bool>`, where
  absent means off, and `idle_compaction_lifetime: Option<IdleLifetime>`,
  `30m` or `1h`, where absent means `1h`.

What older pieces meet:

* An older page ignores the new fields: it shows no rows and the usual
  divider.
* An older backend's entries have no `cache_ttl` until the migration, which
  keeps what each sends.

## Spike

* **What each vendor keeps.** Settled by the vendors' documentation, read
  October 2026:

  | Vendor                            | Default                                            | Can ask for          |
  | --------------------------------- | -------------------------------------------------- | -------------------- |
  | Anthropic                         | 5 minutes                                          | 1 hour, at 2× writes |
  | OpenAI, GPT-5.6 and later         | at least 30 minutes                                | 12 hours, at 2.5× writes |
  | Kimi K3                           | 5 minutes                                          | 1 hour               |
  | Claude Code                       | 1 hour within plan usage, 5 minutes beyond it      | 5 minutes, 1 hour    |
  | Qwen                              | 5 minutes explicit; implicit undefined             | nothing              |
  | DeepSeek                          | hours to days, undefined                           | nothing              |
  | Gemini, Grok, MiniMax, GLM, Doubao | undocumented                                      | nothing              |

* **Whether Codex takes `prompt_cache_options.ttl`.** It does not. A request
  with `12h` through a Codex account on a GPT-6 model came back HTTP 400,
  `invalid_parameter`: "prompt_cache_options is not supported on this
  model". Codex offers no row and keeps the 30-minute default.

## Prior art

Claude Code compacts an idle conversation (`idleCompaction` in its
[settings reference](https://code.claude.com/docs/en/settings-reference.md#idlecompaction)).
It is on by default, for an interactive session on the one-hour cache, from
200,000 tokens of context, about 54 minutes after the last request. Its
terminal compacts then and shows *Compacted while idle, before the prompt
cache expired*. Its VS Code panel and desktop app write the summary then,
and use it only if the next message comes after the hour.
`promptCacheTtl` chooses `5m` or `1h` for the main conversation.

Demi departs in these ways:

* It is off by default.
* A minimum of 30 minutes may be chosen, so an OpenAI conversation on its
  default qualifies.
* The time is a share of the lifetime, not a fixed 54 minutes, so a 12-hour
  cache is not compacted while it still holds.
* It compacts only on models whose window is at least 300,000 tokens.
* The lifetime is chosen per provider entry, where its parameter belongs.

## Alternatives

* **Write the summary while idle and use it only when the next message
  comes after the cache expired**, as Claude Code's VS Code panel does. The
  conversation would keep its whole history when the user returns in time,
  but a summary written and not yet used is a state of its own, stored
  across restarts and shown nowhere. Not chosen.
* **A fixed one-hour minimum.** It leaves out every OpenAI conversation on
  its default. The user chose the setting.
* **A cache lifetime per model, in the model dialog.** It could disagree with
  what the request sends: an entry that sends 5 minutes, with a model that
  says an hour, would be compacted at minute 54 from a cold cache, at full
  price, for nothing.
* **Assume the vendor's default for an API-key entry with no lifetime.** It
  keeps the setting's meaning implicit: a user who never chose a lifetime
  would see conversations compacted. An API-key entry compacts only on a
  lifetime its user chose.
* **The lifetime the vendor reports writing, for every provider.** Only
  Anthropic's usage and OpenAI's GPT-5.6 and later report it, so every other
  vendor would never compact. Demi uses it only for Claude Code, which
  chooses for itself.

## Open questions

None.
