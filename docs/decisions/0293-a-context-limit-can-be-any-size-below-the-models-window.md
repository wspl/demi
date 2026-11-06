---
status: proposed
---

# A context limit can be any size below the model's window

## Problem

A user limits the context Demi uses for a model, and a subagent profile
names a Context of its own, from three sizes: 500K, 300K and 200K, offered
only on a window of 500K or more. A product that wants a 1M model to
compact at 350K, or a 500K one at 272K, has no way to say it: the menu
offers neither, the settings route refuses both, and a stored size outside
the three is taken as no limit at all.

The three sizes are the model menu's, a short list to pick from; Demi uses
a limit only to place its compaction thresholds and its usage indicator,
which work at any size, as they do for a model whose own window is small.

## Decision

A limit is any whole number of tokens below the model's window, and a
profile's Context any up to it:

* **What applies.** A stored limit applies while it is below the model's
  window, else the full window does, as now for a window that stops
  offering a size. `applied_context_limit` says so in the core and, through
  the generated lookups, on the page.
* **What is stored.** `PATCH /settings/preferences` takes any `tokens` of
  at least 1, as does a profile's `contextLimit`.
* **Words.** `demi agent profiles --context` and a workflow's `context:`
  take `default`, `full`, or a size such as `350K`, `1M` or `272000`.
  A size above the model's window is refused with the window.
* **A command.** `demi providers context <entry> <model> <size>` sets the
  user's limit on a model, `full` to remove it, as the model menu's
  Context row does, with the user's permission as `demi providers hide`
  has. Without it an agent could set no limit at all.
* **The page.** The Context row and a profile's Context still offer the
  three sizes. A size stored outside them shows as one more item, checked,
  in its place by size, so the menu says what applies; a model under 500K
  shows the row once it holds such a size. Two sizes that would read alike,
  such as a limit of 1,000,000 beside a window of 1,048,576, both 1M,
  read whole.

```text
Context   1M
          500K
        ✓ 350K
          300K
          200K
```

## Spike

None: the compaction thresholds and the usage indicator read the window
in use whatever its size, as for models whose own window is small.

## Prior art

* Codex's `model_context_window` and `model_auto_compact_token_limit`
  take any number of tokens
  ([Codex](https://github.com/openai/codex/blob/main/docs/config.md)).
* OpenCode's model `limit.context` is any number
  ([OpenCode](https://opencode.ai/docs/models/)).

## Alternatives

* **More sizes in the menu**: every product's number would need its own
  step, and the list would grow without end.
* **A Custom field in the menu**: a text field in a menu is a pattern no
  macOS menu has; a product that needs another size sets it once through
  the command or the route.

## Open questions

None.
