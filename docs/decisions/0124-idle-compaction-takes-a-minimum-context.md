---
status: proposed
---

# The context an idle conversation compacts from is a setting

Supersedes: 0107 (An idle conversation compacts before its cache expires),
its fixed 200,000-token minimum context.

## Problem

Alex works with a model whose window is 400,000 tokens, on a provider that
keeps the prompt cache for an hour. Alex's conversations often sit at about
150,000 tokens when Alex leaves for a meeting. They never compact while
idle, because the minimum is a fixed 200,000 tokens, so every return
writes 150,000 tokens to the cache again. Riley, on the other hand, wants
only conversations of 300,000 tokens or more summarized, and cannot ask
for that either.

## Decision

Settings › Agent › Context gains a row under Minimum Cache Lifetime:

```text
┌──────────────────────────────────────────────────────────────────────┐
│ Compact idle conversations                                       [●] │
├──────────────────────────────────────────────────────────────────────┤
│ Minimum cache lifetime                       [30 Minutes | 1 Hour ]  │
├──────────────────────────────────────────────────────────────────────┤
│ Minimum context size                                    [ 200    K ] │
│ Only conversations whose context is at least this large are          │
│ compacted.                                                           │
│ ⓧ Enter at least 100K.                       (only for a value below) │
└──────────────────────────────────────────────────────────────────────┘
```

- The field takes a token count in thousands or millions, as a model's
  context window does in the model dialog (`TokenInput`). It is 200K until
  the user changes it, and is disabled, keeping its value, while the switch
  is off.
- It commits on Enter or when it loses focus, and Escape gives it back the
  value the setting holds without closing Settings. A value below 100K, or
  no value, stays in the field with *Enter at least 100K.* under the row,
  and the setting keeps its value.
- The message takes a line of its own under the row (`SettingsRow`'s
  `problem` slot), so the field stays where it was. A subagent limit's
  field reports its problems there too, where they moved its field up.
- Settings' search finds the row by its name and by words such as
  *context*, *tokens* and *threshold*.
- The other conditions of 0107 are unchanged: the switch, a window of at
  least 300,000 tokens, a known cache lifetime of at least the minimum, an
  idle conversation, and nine tenths of the lifetime passed.

The design:

- `Preferences` and its patch gain `idle_compaction_context: Option<u64>`,
  in tokens, where absent means 200,000. The backend refuses a patch below
  100,000 with HTTP 400.
- The session reports a due time from 100,000 tokens carried, the least
  the setting takes, and says how many it carried:

  ```rust
  pub struct IdleCompactionDue {
      pub at: Timestamp,
      pub lifetime: Duration,
      /// The tokens the request carried, input and cache alike.
      pub carried: u64,
  }
  ```

- The shard stores `carried` beside the due time in `idle_compactions`, and
  compares it with the user's minimum when the time comes, as it compares
  the lifetime: a change of the setting applies to the times already
  stored. A migration gives the rows stored before it 200,000, the least
  they qualified with.

What older pieces meet: an older page shows no row and sends no value, so
the minimum stays 200,000.

## Spike

None.

## Prior art

Claude Code reads its minimum from the environment variable
`CLAUDE_CODE_IDLE_COMPACT_MIN_TOKENS`: 200,000 by default, any positive
whole number, and a value below 100,000 is raised to 100,000. Demi has the
same default and the same floor, and departs in these ways:

- It is a row in Settings, not a variable, because Demi's preferences are
  the user's and are set on the page.
- A value below the floor is refused where it is typed, not raised in
  silence, so the setting never holds a number other than the one shown.

## Alternatives

- **Presets (100K, 200K, 300K) in a segmented control,** as the lifetime
  row has. Fewer choices than the variable gives, and a window can be any
  size.
- **No floor.** Below 100,000 tokens a cache write costs little next to the
  detail a summary loses, and a timer would be armed for nearly every
  conversation.
- **Raise a value below the floor to 100K,** as Claude Code does. The field
  would show a number the user did not type, with no word why.

## Open questions

None.
