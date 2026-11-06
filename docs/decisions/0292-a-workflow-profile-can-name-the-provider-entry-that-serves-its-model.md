---
status: proposed
---

# A workflow profile can name the provider entry that serves its model

## Problem

A workflow's profile names a model by its id, and the first of the user's
provider entries that offers that id serves it. A user with two entries of
the same model, such as `claude-sonnet-5-5` through an Anthropic key and
through a company's compatible gateway, cannot say which one a profile's
children run on: the first in the list always does, and reordering the
list to move one profile moves every profile and the user's own menus.

## Decision

A profile takes an optional `provider`, one of the user's provider entries
by its name, as `demi providers list` shows it and Settings › Providers
names it, or by its id:

```yaml
profiles:
  - id: reviewer
    description: Reviews a change before it is committed.
    model: claude-sonnet-5-5
    provider: Company Gateway
```

```rust
pub struct WorkflowProfile {
    // ...
    /// The name or id of the user's provider entry that serves the model;
    /// none for the first entry that offers it.
    pub provider: Option<String>,
}
```

* It needs `model`, as `effort`, `tier` and `context` do.
* With it, that entry serves the model; without it, the first entry that
  offers the model does, as now.
* It finds the entry as `demi agent profiles create --provider` does, with
  the same code: an id first, else the one entry of that name. A name two
  entries share is refused with both ids, since names are the user's to
  repeat.
* An entry it doesn't find, one that is hidden, or one whose model is
  hidden or cannot be used, makes the profile unavailable with the reason,
  as a model no entry offers does now: `demi agent profiles list` shows
  it, and its spawns fail with it. `profiles list` and `show` name the
  entry a workflow's profile runs on, as they name a user's profile's.
* A name first: the file is written and read by people and kept in a
  repository, and an entry's id is a random string no page shows. An entry
  renamed breaks the profile with the reason, which the file's author
  fixes; the id serves where two entries share a name.

The format is the same for every workflow file, which reads the same
parser.

## Spike

None: the profile's model is already resolved against the user's entries
when the workflow is read; the name narrows the entries it looks in.

## Prior art

* OpenCode names a model as `provider/model`, with the provider's id the
  user chose in its configuration
  ([OpenCode](https://opencode.ai/docs/models/)).
* Codex's profiles name a `model_provider` by its key in the configuration
  ([Codex](https://github.com/openai/codex/blob/main/docs/config.md)).

Both name the provider by a name the user wrote; here that name is the
entry's, which the user sets in Settings.

## Alternatives

* **The entry's id alone**: stable across renames, but unreadable in a
  file a team shares, and found only through `--json`.
* **`provider/model` in `model`**: an entry's name can hold a slash, and
  one field would then mean two things.

## Open questions

None.
