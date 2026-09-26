# Contracts

Every message and stored document that crosses a process boundary is defined
once, as a Rust type. For example, adding a `durationMs` field to a transcript
block works like this:

1. A developer adds the field to the `Block` type in the `core` crate, with its
   serde and garde attributes.
2. `bun run contracts` builds the workspace and runs `xtask contracts`, which
   rewrites the generated TypeScript in `packages/protocol/src/generated/`.
3. `bun run typecheck:web` reports every place in the frontend that the new
   field breaks.
4. The backend validates the field when it decodes a block, and the browser's
   generated Zod schema validates it when a block arrives.

There is no second declaration to forget. The Rust type is the only
definition; its JSON Schema, its TypeScript type and its browser-side
validator are derived from it.

## Contract crates

A contract crate holds the types of one wire or data family, their serde,
schemars and garde attributes, and the decode function of each boundary that
receives them. Both ends of a wire link it, or the TypeScript generator reads
it. A contract crate has no async runtime and no IO, with one exception:
`command-service` carries the SDK that speaks the command wire next to the
wire's types. [Crates and packages](crates-and-packages.md#contract-crates)
names each crate's items.

| Wire or stored data | Contract crate | Ends |
|---|---|---|
| Browser HTTP requests and responses | `web-api` | Backend; `web`, through generated TypeScript |
| Conversation WebSocket frames, transcript blocks and patches, tool views | `agent-protocol`, `core` | Backend; `agent-client` and `web-ui`, through `@demicodes/protocol` |
| Runner wire (MessagePack over a WebSocket) and command manifests | `runner-protocol`, with manifest nodes from `command-tree` | Backend; runner |
| Managed boot record | `runner-protocol` | Backend and machine manager; the runner in a Cloud sandbox reads it |
| Command invocations between a runner and a command program | `command-service` | Runner; `demi-commands`, `demi-claude` |
| `demi.builtin` operations, live view messages, capture extension events | `builtin-protocol` | `coding-agent` declarations, the backend and `demi-commands`; the page reads live view messages through `@demicodes/protocol` |
| `demi.claude` operations and the Claude Code release record | `claude-protocol` | Backend; `demi-claude` |
| Machine-manager socket (one JSON document per line over a Unix socket) and the Cloud image manifest | `machines-protocol` | Backend; machine manager; `xtask` writes the image manifest |
| JSON stored in the control and conversation databases | The crate that owns the data, such as `core` for blocks | Backend |

A wire whose two ends are both Rust needs no generation: both ends link the
same crate. The runner wire and the machine-manager socket have fixed
encodings, and a golden corpus with one message of every kind pins their bytes
at both ends. Integer fields travel as integers. Vendor APIs are not Demi
contracts: each provider crate declares only the parts it reads
([Providers](../providers/providers.md)).

A shape is declared once. Nothing restates a contract type by hand, in Rust or
in TypeScript: a second declaration of the same shape is a defect, even while
the two still agree.

### Encoding conventions

Browser-facing types and JSON stored by the backend follow one serde
convention:

- Fields are camelCase.
- Enums are internally tagged: by `type`, or by `op` for transcript patches,
  `status` for nested outcomes and `kind` for views. No untagged enum exists
  anywhere; the TypeScript generator rejects them.
- An optional field is omitted when absent, and a `null` in its place is
  refused (`Option<T>` with `default`, `skip_serializing_if` and serde_with's
  `unwrap_or_skip`). A nullable field is always written, as `null` when empty,
  and its absence is refused (`Option<T>` decoded with `Option::deserialize`).
  The generated Zod schemas refuse the same values, so both ends agree.
- Bytes are base64 strings (`B64Bytes`). Times are RFC 3339 strings in UTC
  with three fractional digits, such as `2026-09-21T14:13:20.000Z`, so that
  the text of two times orders as the times do (`core`'s `Timestamp`, whole
  milliseconds of a `jiff::Timestamp`); a finer time is refused.
- Integers are integer types. An integer the browser reads is bounded to
  JavaScript's safe integer range in the Rust type as well: a 64-bit field
  carries garde's `range(max = MAX_SAFE_INTEGER)`, core's constant, so an end
  that decodes it refuses what the browser cannot hold, and generation fails
  for a field without the bound. On a type only the backend sends, nothing in
  Rust checks the attribute: it states the bound the browser's schema checks.

The runner wire has its own field names and MessagePack encoding, fixed by
`runner-protocol` and its corpus; its optional and nullable fields follow the
rule above. [Storage](../backend/storage.md) owns the
storage encodings: column types, times in the database, digests, sealed
credentials and password hashes.

## Validation at entry

When a runner sends `job_exit`, the backend hands the MessagePack bytes to
`runner-protocol`'s decode function. Serde decodes them into the `Outbound`
enum, so an unknown message type, a missing field or an exit code that does
not fit its integer type fails there; garde then checks bounds. The connection
code receives either a typed message or an error that names the field, and a
malformed message closes the connection. Nothing after the decode function
checks the message again, and nothing before it looks inside.

Every value from outside a process follows the same rule:

- Each boundary has one decode function per message family. Serde decodes the
  value into its type: closed sets as enums, identities as newtypes that
  cannot hold an invalid value, integers as integer types. Then garde checks
  bounds, lengths and patterns and reports the field path.
- A rule across fields is the type's shape, or a conversion from a raw form
  (`try_from`) inside the decode.
- A field the contract declares opaque, such as a tool call's input, stays a
  JSON value until its owner validates it.
- Each end validates what it receives. A value is valid because a decode
  function checked it, not because it has a type: no code converts unchecked
  input into a contract type, and corrupt data is refused, never repaired or
  defaulted.
- Fixed contracts are validated by their types and garde, never by
  round-tripping a value through JSON or a JSON Schema. JSON Schema validation
  belongs to command arguments, whose schema travels in the manifest: both
  ends check it with `jsonschema` and word a failure the same way
  ([Parse input and render help](../execution/commands.md#parse-input-and-render-help)).

A string length counts Unicode scalar values, garde's `chars` mode, wherever
a bound is checked: in the browser's schemas and in command inputs alike.
Zod 4 counts code points and JSON Schema counts characters, which are the
same for every string serde accepts, while garde's default counts bytes and
JavaScript's `length` counts UTF-16 code units. So the one attribute gives
both ends the same limit. For example, `😀` is one scalar value and two UTF-16
code units, so a file name of 255 of them fits a limit of 255
([Commands](../execution/commands.md)). Truncation and token estimates have
their own units, defined with the rules that use them
([Token estimates](../agent/compaction.md#token-estimates)).

These are the points where values enter, and what a failure does:

| Where a value enters | Decoded by | When it fails |
|---|---|---|
| A browser request body or query | The edge's body and query extractors, into `web-api` types | 400 `invalid_body` or `invalid_query`, naming the field and the reason ([Web API](../product/web-api.md)) |
| A frame on the conversation WebSocket | The conversation socket, into `ClientFrame` | An `error` frame with code `invalid_frame`, before any state changes; a message that is not JSON closes the socket ([Frame protocol](../agent/runtime.md#frame-protocol)) |
| A frame or REST response the browser receives | The generated schemas, in `agent-client` and `web` | `agent-client` drops the connection and reports the field path; `web` validates a response before applying it to state |
| A runner message, at either end | `runner-protocol`'s codec | The connection closes ([Runner](../execution/runner.md)) |
| Invocation metadata and records between a runner and a command program | `command-service` | [Validation and flow control](../execution/native-runtime.md#validation-and-flow-control) |
| A command's arguments | The declaration's JSON Schema, at the dispatcher and again in a native handler before work | One usage error that names every field that failed |
| A machine-manager request or response | `machines-protocol` | A malformed line or an unknown operation drops the connection; an invalid device id is that operation's error ([Managed Cloud hosts](../cloud/managed-hosts.md)) |
| The managed boot file | `runner-protocol`'s `ManagedBoot` | The runner fails; it never falls back to pairing ([Runner](../execution/runner.md#managed-guests-and-verification)) |
| A capture extension event | `builtin-protocol` | The extension connection fails, and the failure is logged ([Live view](../browser/live-view.md)) |
| A row or JSON column read from a database | The backend's storage module | The restore stops; nothing is repaired or defaulted ([Storage](../backend/storage.md)) |
| A sealed credential document | The vault | The error names the field path and the kind of failure, never the value ([Providers](../providers/providers.md)) |
| Configuration from arguments and the environment | Each program's configuration, at startup | The program does not start, and the error names the variable |
| A tool call's input from the model | The tool | The model receives the tool's error ([Tools](../agent/runtime.md#tools)) |
| A vendor API response | The provider crate's two-step decode | An unknown `type` is skipped; a known one with a malformed payload is a protocol failure that is never retried automatically ([Providers](../providers/providers.md)) |

## Generated TypeScript

The browser's contract types and validators are generated from the Rust types:

```text
Rust type (serde + schemars + garde)
   |  schemars: the JSON Schema of serde's actual representation, in memory
   v
xtask contracts: the emitter, which fails on anything outside its subset
   |
   v
Zod source and z.infer types
   -> packages/protocol/src/generated/   @demicodes/protocol: core, agent-protocol,
                                          builtin-protocol types the page reads
   -> packages/web/src/api/generated/    web: the web-api REST types
```

- **Generation.** `bun run contracts` builds the workspace with its one Cargo
  selection and runs `target/debug/xtask contracts`. Generated files are not
  committed; the scripts that need them (`typecheck`, `typecheck:web`,
  `test`) run generation first. Cargo builds never run the emitter and need
  no JavaScript tooling.
- **What is emitted.** The emitter starts from a list of root types in
  `xtask`, each with what the browser does with it: receives it or sends it.
  Every type a root refers to is emitted with it. The roots of
  `@demicodes/protocol` are core's types, the socket's frames and the live
  view's messages; its schemas and types are in `generated/contracts.ts` and
  its tables in `generated/tables.ts`, which the package's entry re-exports.
  The roots of `web` are the web-api request and response bodies; its
  `src/api/generated/web-api.ts` imports the schemas it shares with
  `@demicodes/protocol` from there. A new body type is added to the roots with
  its direction.
- **One constraint definition.** A garde attribute drives both the Rust check
  and the emitted schema: schemars reads garde's attributes, including
  `length(chars, ...)`, and emits `minLength` and `maxLength`. Internally
  tagged enums become `oneOf` with `const` tags, which the emitter turns into
  discriminated unions.
- **Supported subset.** Objects; internally tagged enums, as discriminated
  unions, including a newtype variant of a struct, which becomes the struct
  extended with the tag; string enums and literals; arrays and records;
  optional fields (`.optional()`, which refuses `null`) and nullable ones
  (`.nullable()`, which must be present); string lengths and patterns;
  integer and number bounds, an integer's within JavaScript's safe range;
  base64 bytes (`z.base64()`); times as core's
  `Timestamp` writes them (`z.iso.datetime({ precision: 3 })`: UTC with three
  fractional digits, the contract's one spelling); email addresses; `http`
  and `https` URLs (`z.url` with those protocols, `web-api`'s `EndpointUrl`);
  text the backend trims on arrival, whose bounds count what the trim leaves
  (`z.string().trim()`, `web-api`'s `Trimmed`); JSON values (`z.json()`);
  flattened plain structs (merged properties); one
  named instantiation of a generic root type; recursion through `$ref`, as a
  getter of the object property that refers back, which Zod types
  recursively; strict and tolerant objects; and constant tables with
  generated lookups: the file-type table, the file types a model reads and
  the live view's frame constants. A pattern must not use what Rust's `regex`
  and the browser's engine read differently: `\d`, `\w`, `\s`, `\b`, `.`,
  groups other than `(?:`, Unicode properties or Rust's class syntax; the
  browser matches patterns by code point (the `u` flag), as Rust does.
  Anything else fails generation and names the type and the place.
- **Rules only Rust checks.** garde `custom` rules are not in the schema and
  are not emitted. Such a rule is checked by the backend alone.
- **Strict and tolerant objects.** Each end judges what it receives. In Rust,
  a type the backend receives refuses unknown fields (`deny_unknown_fields`).
  That includes the types both ends receive, because the backend's check
  guards its state: a model selection arrives in `open` and travels in every
  block, and a block is read back from the conversation database. In the
  browser, the schema of every type the browser receives is a tolerant
  object, including the blocks and selections inside server frames: a page
  left open across a deploy that adds a field keeps working and ignores the
  field. The emitter writes a strict object only for a type the browser never
  receives, such as a client frame, and refuses to generate one whose Rust
  type accepts unknown fields: the backend receives it.
- **Tolerant values where the browser receives.** The same rule decides how
  closely a value's schema follows its Rust type. Where the browser only
  receives a value, its schema may accept more than the type holds, since the
  backend never sends the difference: `z.base64()` accepts the nonzero
  padding bits that `B64Bytes` refuses, a failure map's keys may be empty
  where a block id cannot, and an email address may carry capitals, which
  `EmailAddress` lowercases. Where the browser sends a value, its schema
  refuses whatever the backend refuses: a name the backend trims is trimmed
  before its length is checked, so blank text is refused, and an endpoint must
  be an `http` or `https` URL. An email address the browser sends passes its
  schema only when it has no surrounding white space and has the form the
  backend checks, so the backend accepts every address the schema does.
- **Checked on arrival.** `agent-client` validates every frame it receives, and
  `web` validates every REST response before applying it to state.
- **One patch applier.** Transcript patches have one applier,
  `applyTranscriptPatches` in `agent-client`. The agent's Rust tests write
  patch sequences together with the snapshot each must produce, and
  `agent-client`'s tests apply them, so no second applier exists in Rust.

The emitter lives in the repository because no available generator covers
this job: `json-schema-to-zod` is unmaintained; `zod_gen` and `schemars-zod`
are small single-maintainer crates; Zod's own JSON Schema import is
experimental and builds validators only at runtime; `ts-rs` and `specta` emit
types without validators.

## The TypeScript boundary

Demi has no TypeScript SDK. The design keeps one possible without embedding
the Rust runtime: a TypeScript program would be a client of serializable
protocols, never a host of the Rust code. Two such protocols exist:

- **The agent frame protocol.** `AgentClient` drives a session through it, as
  the browser does.
- **Application commands.** An `rpc` command leaf is a callback to the
  application that declared it. Another process could register such leaves
  and serve their invocations, so a command could be written in TypeScript.

The constraint that keeps this possible: everything an `rpc` handler receives
is expressible as messages. That covers its arguments, byte IO, working
directory, environment, cancellation, command storage, and Host operations
once a handler needs them. The command system dispatches through `RpcHandler`
with a serializable `RpcInvocation` and an `RpcPort` whose every operation is
a message, and in-process handlers implement the same interface. A handler
never receives a live object, such as a Host handle or a callback into the
backend. This is why command storage changes by versioned compare-and-set
rather than by a callback inside a transaction
([Command state history](../agent/command-state-history.md)). The port offers
no Host operation until a handler needs one.

## Logic the browser and backend share

Logic that both the browser and the backend need gets one owner, chosen case
by case:

| Logic | Owner | How |
|---|---|---|
| The file-type table: which files the product previews, by extension, and which the page shows in place | `core` | The page must choose a viewer before any byte arrives ([Choosing a view](../product/file-previews.md#choosing-a-view)), so the table and its lookups are emitted into `@demicodes/protocol`, and the backend serves files by the same definition |
| Whether an upload is text, and its short opening snippet | Backend | The upload response carries the snippet the composer's tile shows |
| The media type a model receives for an upload | Backend | The upload response carries the sniffed media type; the message editor uploads files the way the main composer does |
| Whether a message can be edited | The data model | `User` is the only editable block type; hidden inputs are `Context`, `Wakeup` and `AgentMessage` blocks |
| The summary text of a queued message | Browser | The queue carries each message's content, and `web-ui` derives the summary |
| Completion message ids | Backend | The rule that ties an id to its sender and round is a garde `custom` check, which is not emitted; the browser receives ids as data |
