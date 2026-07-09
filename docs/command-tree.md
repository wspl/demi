# Registered Command Tree

Final-state design for the registered-command model in `@demicodes/shell`.  
Scope: **command tree only**. Command bridge is a later design. Native tool projection is out of scope and treated as accidental surface to remove, not extend.

**Base:** `abe8fee` (pre–command-bridge).  
**Constraint:** one-shot rewrite — no compatibility shims, no dual type systems, no “old shape still works” paths.

---

## Problem

Registered commands are the product CLI surface (e.g. `editor create`, future product namespaces like `larkclaw watch create`). The model at `abe8fee` is already a recursive tree, but nodes are a **discriminated union**:

| Kind | Type | Can route? | Can run? |
|------|------|------------|----------|
| Group | `CommandSpec` | yes (`subcommands` required) | **no** |
| Leaf | `CommandSubcommandSpec` | no | yes |

That cannot express:

1. **Bare top-level command** — `kcenv <key>` (root runs with its own args; no fake subcommand).
2. **Runnable group** — a node that both has children and accepts its own flags/positionals (standard cobra/click/commander shape).

Routing and execution are independent capabilities. Forcing “exactly one” is the bug.

---

## Goals

1. One node type for the whole tree.
2. A node may **route**, **run**, or **both**; registration rejects “neither”.
3. Parse order matches real CLIs: **child name wins**, else treat remaining argv as this node’s args (if runnable).
4. Arbitrary depth; top-level registry entries are just root nodes.
5. Help (`prompt`) at every node that can be a routing stop.
6. Full rewrite of all in-repo definitions and call sites; delete obsolete names.

## Non-goals

- Command bridge (PATH shims / UDS) — separate design after this lands.
- Native tool projection — remove coupling; do not redesign.
- Streaming, async job APIs, or a second dispatch path outside `runRegisteredCommand`.
- Preserving `CommandSpec` / `CommandSubcommandSpec` / `isCommandGroup` as aliases.

---

## Core model

### Single type: `Command`

```ts
export type CommandInputSpec = Record<string, z.ZodType>

export interface CommandOutputSpec {
  json?: z.ZodType
}

/**
 * One CLI tree node. Routing (`subcommands`) and execution (`run`) are
 * independent optional capabilities. Registration requires at least one.
 */
export interface Command {
  name: string
  summary: string

  /** Present when this node routes to named children. */
  subcommands?: Command[]

  /**
   * Present when this node is executable with its own args/flags.
   * Execution-only fields below are only meaningful when `run` is set;
   * register() rejects `run` without `examples`, and rejects execution
   * fields without `run`.
   */
  run?: (ctx: CommandRunContext) => Promise<CommandRunResult> | CommandRunResult
  effects?: string
  successOutput?: string
  failureOutput?: string
  input?: CommandInputSpec
  positionals?: string[]
  stdinField?: string
  output?: CommandOutputSpec
  /** Required iff `run` is set. */
  examples?: string[]
}
```

**Why not `leaf: { run, input, ... }`?**  
Yesterday’s redesign nested execution under `leaf`. That encodes the same independence but forces every product command into an extra object and keeps “group vs leaf” as a mental split. Flattening `run` onto `Command` matches cobra/commander (the command *is* the unit) and makes bare leaves the obvious default:

```ts
// bare root
{ name: 'kcenv', summary: '...', positionals: ['key'], examples: [...], run: ... }

// pure group
{ name: 'editor', summary: '...', subcommands: [ create, edit, ... ] }

// dual-mode (allowed)
{ name: 'tool', summary: '...', subcommands: [...], input: {...}, examples: [...], run: ... }
```

**Deleted types / helpers**

- `CommandSpec`
- `CommandSubcommandSpec`
- `CommandNode` as a union type
- `isCommandGroup`

Public renames (breaking, intentional):

| Old | New |
|-----|-----|
| `CommandSpec` | `Command` |
| `CommandRegistry.register(spec: CommandSpec)` | `register(command: Command)` |
| `list(): CommandSpec[]` | `list(): Command[]` |
| `get` / `parseCommandInput` / `runRegisteredCommand` / `renderCommandPrompt` first arg | `Command` |

Adapter rename for clarity (same file, same role):

| Old | New |
|-----|-----|
| `commandSpecToForkCommand` | `commandToForkCommand` |

### `ParsedCommandInput`

```ts
export interface ParsedCommandInput {
  /**
   * Path from root through the selected node, including the root name.
   * Examples: ['editor','create'] | ['kcenv'] | ['lark','watch','create']
   * For help: path of the node help was requested for (may be a pure group).
   */
  path: string[]
  /** True when the invocation was `<path…> prompt`. */
  help: boolean
  values: Record<string, unknown>
  json: boolean
}
```

**Removed:** `subcommand: string` — redundant with `path.at(-1)` and wrong for bare roots / help-on-group. Call sites that branched on `parsed.subcommand === 'prompt'` use `parsed.help`. Call sites that only needed the leaf name use `path[path.length - 1]`.

---

## Parse algorithm

Input: root `Command` R, `argv` where `argv[0] === R.name`, optional stdin.

```
node := R
path := [R.name]
i := 1

loop:
  if i >= argv.length:
    // no more tokens
    if node.run: return parseArgs(node, path, argv, i, stdin)  // bare invocation of runnable node
    else: error "requires a subcommand"

  token := argv[i]

  if token === 'prompt':
    // help is only legal as a free token at a routing boundary (not after
    // a leaf has started consuming positionals/flags). We only reach here
    // while still walking.
    return { path, help: true, values: {}, json: false }

  if node.subcommands has child named token:
    node := child
    path.push(token)
    i += 1
    continue

  // token is not a child name → stop walking; remaining argv belong to node
  if !node.run:
    error unknown subcommand / not runnable
  return parseArgs(node, path, argv, i, stdin)
```

`parseArgs` is the existing flag/positional/stdin/`--json` logic, scoped to `node.input` / `positionals` / `stdinField`. Display paths in errors use `path.join(' ')`.

### Disambiguation (normative)

| Situation | Behavior |
|-----------|----------|
| Token matches a child name | Always enter that child (even if parent is runnable and could take it as a positional). |
| Token is `prompt` while still walking | Help for **current** node; do not treat as positional. |
| Token is `--…` or non-child positional on a runnable node | Stop walk; parse as this node’s args. |
| Walk ends on a pure group with no more tokens | Error: requires a subcommand. |
| Walk ends on a pure group with a non-child token | Error: unknown subcommand. |
| Child named `prompt` | **Illegal** at register time at every level. |

This is cobra’s “commands first, then flags/args” order.

### Examples

```text
editor create src/a.ts          → path [editor,create], run create
editor prompt                   → path [editor], help
editor create prompt            → if create is a pure leaf, "prompt" is a positional/flag error
                                 (not help), unless create has a child named prompt (forbidden)
kcenv MY_KEY                    → path [kcenv], values.key=MY_KEY
tool --x 1                      → path [tool], tool.run with --x (dual-mode parent)
tool sub --x 1                  → path [tool,sub], sub.run
```

---

## Registration validation

On `CommandRegistry.register(root)` (and recursively for every node):

1. `name` non-empty; root `name` not in `RESERVED_COMMAND_NAMES`.
2. Root name unique in the registry.
3. Every node: `Boolean(run) || (subcommands?.length ?? 0) > 0`.
4. If `run` is set: `examples` is a defined array (may be empty only if we choose to allow — **decision: require `examples: string[]` present when `run` is set**; empty array allowed for test fixtures).
5. If `run` is absent: reject any of `input`, `positionals`, `stdinField`, `output`, `effects`, `successOutput`, `failureOutput`, `examples` (no silent dead fields).
6. Sibling names unique under a parent; no child named `prompt`.
7. `stdinField`, if set, must be a key of `input`.
8. Every `positionals` entry must be a key of `input`.

No partial registration: throw before mutating the map if validation fails.

---

## Run path

`runRegisteredCommand(root, ctx)`:

1. `parsed = parseCommandInput(root, ctx.argv, stdin)`.
2. If `parsed.help`: resolve node by `parsed.path`, `renderCommandPrompt(node, …)`, stdout, exit 0.
3. Else resolve node by `parsed.path` (must have `run`), optional JSON capture/validate (existing behavior), call `node.run(ctx)`.

Resolve-by-path is a simple walk: for each segment after the root, find child by name (root is `path[0]`).

---

## Help rendering

- Registry `renderPrompt()`: optional one-line registry-wide defaults (keep current `COMMAND_PROMPT_DEFAULTS` budget idea — defaults once, per-command only deviations), then each root’s tree.
- `renderCommandPrompt(node, qualifiedPrefix)`:
  - Header: qualified name + summary.
  - If `node.run`: emit a **Usage** block for *this* node (parameters, stdin, examples, effects/success/failure only when set).
  - If `node.subcommands`: list each child as a short line (`qualified child` + summary); recurse into children that have further structure or their own `run`.
- Pure groups are navigational headers + child list, not fake “not runnable” leaf entries.
- Fully qualified paths in all usage lines (`editor create`, not bare `create`).

Exact prompt wording can stay close to today’s leaf blocks; structure must not assume “only leaves have run”.

---

## Fork adapter

`commandToForkCommand(session, root, storage)`:

- `name`: `root.name` (still one fork entry per **root**).
- `consumesStdin`: true if **any** runnable node in the tree has `stdinField` (recursive).
- `execute`: `argv = [root.name, ...args]` → `runRegisteredCommand(root, …)` unchanged in spirit.

Nested commands are never separate fork registrations; they are argv under the root. Bridge (later) will symlink **root names only** for the same reason.

---

## Call-site rewrite (complete list)

All of these move to `Command` with flattened `run` (no `leaf` wrapper).

| Area | Files |
|------|--------|
| Core | `packages/shell/src/command.ts`, `registered-command-adapter.ts`, `environment.ts` (`registerCommand` / `registeredCommands` types), `index.ts` exports |
| Projection | **Delete** `command-projection.ts` and agent `command-tools` wiring if present on the branch; do not port to the new model in this work |
| Product commands | `packages/coding-agent/src/editor-command.ts`, `todo-command.ts`, `coding-harness.ts` |
| Agent harness types | `packages/agent/src/types.ts` (`commands?: … Command[]`), `server.ts` comments/types only as needed |
| Tests | All shell/coding-agent/agent fixtures that build command literals |

No temporary `CommandSpec` type alias. Grep for `CommandSpec` / `CommandSubcommandSpec` / `isCommandGroup` must be empty after the change.

### Shape migration examples

**Before (group + leaves):**
```ts
{
  name: 'editor',
  summary: '…',
  subcommands: [
    { name: 'create', summary: '…', input: {…}, examples: […], run: async () => {…} },
  ],
}
```

**After:** same for pure group + pure leaves — only type name changes (`Command`). No structural change for the common case.

**Before (impossible): bare root**

**After:**
```ts
{
  name: 'kcenv',
  summary: 'Read an env value',
  positionals: ['key'],
  input: { key: z.string() },
  examples: ['kcenv HOME'],
  run: async ({ parsed, io }) => { … },
}
```

---

## Testing (acceptance)

Own these cases in `packages/shell` (unit) and thin integration where argv goes through the fork adapter:

| # | Case |
|---|------|
| 1 | Depth-1 group + leaves (`editor create`) — parity with today |
| 2 | Depth ≥2 nested groups |
| 3 | Bare root leaf with positionals/flags |
| 4 | Dual-mode: parent `run` + child; child name shadows parent positional |
| 5 | `prompt` at root and at nested group |
| 6 | `prompt` is not stolen as help once a pure leaf is selected (becomes normal parse error if not a valid arg) |
| 7 | Register rejects: empty node, dead fields without `run`, duplicate sibling, `prompt` child, reserved root name |
| 8 | `--json` validate-on-success path unchanged in spirit |
| 9 | `consumesStdin` true if any descendant has `stdinField` |
| 10 | coding-agent `editor` / `todo` still work through `BashEnvironment.exec` |

---

## Package boundaries

Unchanged: tree types and registry live in `@demicodes/shell`. No agent/host imports. Products (`coding-agent`) only construct `Command` values and register them.

---

## Implementation order (single PR preferred)

User rule: one-shot, no staged compatibility. Prefer **one PR**:

1. Rewrite `command.ts` (types + parse + run + validate + help).
2. Rewrite adapter + environment type edges.
3. Rewrite editor/todo/harness + agent type edges.
4. Delete projection module and its imports/tests if still in tree.
5. Rewrite tests to the acceptance table; remove obsolete union/isCommandGroup tests.
6. Full package typecheck + targeted tests.

If the diff is unreviewably large, split only as:

- **PR1:** `command.ts` + shell tests + adapter (shell package green).  
- **PR2:** coding-agent + agent call sites (no behavior flags).  

No PR that leaves both old and new types exported.

---

## Key decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Node model | Single `Command`; `subcommands?` + `run?` | Independent capabilities; bare leaf and dual-mode natural |
| Execution nesting | Flatten `run` on `Command`, not `leaf: {}` | Less ceremony; matches CLI frameworks; common case unchanged |
| Parse order | Child name first, else this node’s args | Standard CLI; predictable dual-mode |
| Help token | `prompt` only while still walking | Avoids stealing leaf positionals named accidentally; keep reserved child name |
| Parsed identity | `path: string[]` + `help: boolean` | Drop misleading `subcommand` |
| Compat | None | Explicit full rewrite |
| Projection | Delete / ignore | Accidental surface; not part of tree goals |
| Bridge | Out of scope | Depends on stable root `Command` + parse; design next |

---

## Open questions

None blocking implementation. Optional product polish (not required to implement the model):

- Whether dual-mode should be discouraged in prompt text (model might confuse parent vs child) — can document in harness guides later.
- Whether `examples` must be non-empty for production commands (tests may use `[]`).

---

## PR Plan

### PR: `refactor(shell)!: single Command tree node with optional run and subcommands`

- **Depends on:** nothing (branch from `abe8fee` or current reset base).
- **Affects:** `packages/shell` (command, adapter, environment, exports, tests), `packages/coding-agent` (editor, todo, harness), `packages/agent` (harness command types, any fixtures), delete projection/command-tools if present.
- **Description:** Replace group/leaf union with final `Command` model; rewrite parse/run/help/validation; migrate all call sites; no shims.
- **Verify:** shell + coding-agent + agent unit tests; grep clean for deleted symbols.

---

## Relationship to command bridge (preview only)

After this lands, bridge materializes **one PATH entry per registry root name** and forwards `argv` into `runRegisteredCommand`. Nested paths and bare roots already work if parse is correct. Bridge must not reimplement tree walking.
