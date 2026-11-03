---
status: proposed
---

# The command line edits a provider entry as Settings does

## Problem

Alex asks an agent to give the entry *Team gateway* a prompt cache lifetime
of one hour and to switch its protocol to Chat Completions. Settings ›
Providers changes both, but `demi providers edit` takes neither: its options
are a list of its own (`--name`, `--base-url`, `--through`, `--api-key-stdin`,
`--image`, `--video`, `--audio`), written beside the settings the page sends.
The agent can only say it cannot. Each setting an entry gains is defined
three times, in the page, in `ProviderPatch` and in the command's options, and
the command's copy lags: today it lacks an entry's protocol, its prompt cache
lifetime, its searches, its rotation, and the switches that turn an entry or
a model off.

The copies have drifted in their names too. One endpoint is `baseUrl` in the
patch, `endpoint` in the entry the API answers and in `show --json`, and
`--base-url` on the command line; one device is `throughDevice` in the API
and `through` in `show --json`.

## Decision

The entry the API answers is the one definition of a provider entry's
settings, and the command line reads and changes it as the page does.

- `demi providers show <entry> --json` prints the entry as `GET /providers`
  answers it (`ProviderDto`), its fields at the top, and beside them what
  is not the entry's own: `status`, `hidden` (turned off in the user's model
  menu), `accounts` (a subscription's, by label), `modelsFrom` and
  `catalog`, each model the entry offers with every field. `catalog` is not
  `models`, the entry's own list that the patch replaces, which a vendor's
  entry that follows models.dev leaves empty.
- `demi providers edit <entry>` reads a JSON merge patch (RFC 7396) from
  stdin and sends it as `PATCH /providers/{id}`: a field it names takes the
  new value, `null` removes it, and a field it leaves out stays. The checks,
  the refusals and their words are the API's.

  ```sh
  demi providers edit 'Team gateway' <<'EOF'
  {"cacheTtl": "1h", "wireApi": "chat-completions"}
  EOF
  ```

- A field of `show --json` that no patch changes, such as `id`, `kind` or
  `createdAt`, may stay in the patch with its value as shown, so the
  printed entry, edited, goes back as it is. Given another value it is
  refused, naming the field.
- What `show --json` prints beside the entry is left out of the patch
  whatever it holds, as Kubernetes leaves a resource's `status` out of an
  update: `status` and `catalog` change by themselves between a `show` and
  an `edit`, and `hidden` and `accounts` have their own commands. A value
  that differs from the shown one draws a warning that names those
  commands.
- A patch that changes nothing succeeds and says so, as `kubectl patch`
  answers `(no change)`.
- The key never goes in the patch, which a transcript keeps:
  `edit <entry> --api-key-stdin` reads it alone from stdin, as today.
  `{"apiKey": null}`, which holds no secret, removes it as Settings does.
- `list --json` and `remove --json` name what `show --json` names with the
  same words: an entry's `label` and `status`, a model's `displayName` and
  `contextWindow`.
- Every option of `edit` that sets a setting goes: `--name`, `--base-url`,
  `--through`, `--image`, `--video` and `--audio`. Each is a field of the
  patch.
- What is not the entry's has commands of its own, as it has routes of its
  own:
  - `demi providers accounts list <entry>` lists a subscription's accounts;
    `accounts activate` and `accounts order` change which one is in use and
    In Order's order. Signing in stays in Settings, which needs a browser.
  - `demi providers hide <entry> [--model <id>]` and `unhide` turn an entry
    or one of its models off and on in the user's model menu, which Settings
    does with Show in Model Menu: a preference of the user's
    (`hiddenProviders`, `hiddenModels`), not a setting of a shared entry.
- `add` keeps its options, which choose what the entry is (`--vendor`,
  `--family`, `--wire`, `--base-url`), names it with `--label`, the field's
  name, in place of `--name`, and takes the key with `--api-key-stdin`.
  Everything else is set with `edit` afterwards, as the page creates an
  entry first and opens its editor.
- `models search`, `models add`, `models edit` and `models remove` stay:
  they fill a model from models.dev and change one model, and each sends
  the entry's `models` as the patch does.

One name per field: the entry the API answers spells the endpoint
`baseUrl`, as the patch, the stored configuration and `--base-url` do; its
`endpoint` goes. The page's generated types follow.

The design to build from:

```rust
// crates/backend-user-shard/src/shard/provider_commands.rs
struct EditArgs {
    entry: String,
    /// The JSON merge patch, read from stdin unless `--api-key-stdin`.
    patch: Option<String>,
    api_key_stdin: bool,
}
```

- The handler decodes the patch as `ProviderPatch`, after it drops the
  fields of `ProviderDto` that no patch changes and that hold their shown
  values, and refuses any other unknown field by its name. It applies the
  patch through the shard's provider settings, the code the route runs.
- `demi providers` keeps its permission, `providers.manage`, for `edit`,
  `accounts activate`, `accounts order`, `hide` and `unhide`.
- A field the API gains is a field `edit` changes and `show --json` prints,
  with nothing to add to the command.

## Spike

None. A leaf already reads its stdin into a string field
(`crates/command-declarations/src/lib.rs`, `stdin_field`), which the
handler decodes.

## Prior art

- `kubectl patch --type merge -p '{…}'` changes any field of an object by a
  JSON merge patch, and `kubectl get -o json` prints the object it patches
  ([Update API objects in place](https://kubernetes.io/docs/tasks/manage-kubernetes-objects/update-api-object-kubectl-patch/)).
  Kubernetes refuses a change to an immutable field and accepts it
  unchanged, as `edit` treats `id` and `kind`.
- `gh api --input` sends a request's body from a file or stdin, so any field
  the API takes is reachable without an option of its own
  ([gh api](https://cli.github.com/manual/gh_api)).
- RFC 7396 defines the merge patch: `null` removes, an absent field stays,
  and an array is replaced whole
  ([RFC 7396](https://www.rfc-editor.org/rfc/rfc7396)).

Where Demi departs: the key has an option of its own, so that it is piped
rather than written into the patch.

## Alternatives

- An option per setting, as today: every setting is defined twice, and the
  command's copy lags, as the Problem shows.
- `--set path=value` and `--unset path`, as Azure CLI's generic update
  arguments take them
  ([Generic update arguments](https://learn.microsoft.com/en-us/cli/azure/use-azure-cli-successfully-advanced#generic-update-arguments)):
  a list, such as an entry's headers, or a nested object, such as its
  media, has no plain spelling as a path.
- Options generated from the patch's schema: a nested field becomes a long
  flattened name, and `null` has no spelling in argv.

## Open questions

None.
