# Contracts

Adding `durationMs` to a transcript block changes one definition: the Go
struct for that variant in `internal/core`. The field is:

```go
// +demi:range max=9007199254740991
DurationMs uint64 `json:"durationMs"`
```

1. A developer adds the field, its JSON tag and its bound to the Go type.
2. `go generate ./...` rewrites the package's generated Go codecs and
   validators. `bun run contracts` runs that generation and
   `go run ./tools/contractgen -ts`, rewriting the generated TypeScript.
3. `bun run typecheck:web` reports every frontend use the new field breaks;
   the Go type checker checks the backend's uses.
4. The backend's generated decoder checks the field when it reads a block,
   and the web app's generated Zod schema checks it when a block arrives.

The Go type is the only definition. Its encoders, decoders, validation,
TypeScript type and Zod schema are derived from it. Nothing declares a
second contract shape by hand, even while the two would still agree.

## Contract packages

A contract package holds the types of one wire or data family, their tags
and markers, and generated code. It has no IO and no goroutines. Both Go
ends import the same package; TypeScript ends use its generated schemas.
[Packages](crates-and-packages.md) owns package names and responsibilities.

| Wire or stored data | Contract owner | Ends |
|---|---|---|
| Web app HTTP requests and responses | `internal/webapi` | Backend; `web`, through generated TypeScript |
| Conversation WebSocket frames, transcript blocks and patches, tool views | `internal/framewire`, `internal/core` | Backend; `conversation-client` and `web-ui`, through `@demicodes/protocol` |
| Runner wire (MessagePack over a WebSocket) and command manifests | `internal/runnerwire`, with manifest nodes from `internal/declare` | Backend; runner |
| Managed boot record | `internal/runnerwire` | Backend and machine manager; the runner in a Cloud sandbox reads it |
| Command invocations between a runner and a command program | `internal/commandwire` | Runner; `demi-file`, `demi-browser`, `demi-claude-code` |
| `demi.file` operations | `internal/cmdpkg/file/fileop` | File plugin declarations; `demi-file` |
| `demi.browser` operations, live view messages, capture extension events | `internal/cmdpkg/browser/browserop` | Browser plugin declarations, backend and conversation browser packages; the page through `@demicodes/plugin-browser` |
| `demi.claude-code` operations and the Claude Code release record | `internal/cmdpkg/claudecode/claudecodeop` | Backend; `demi-claude-code` |
| Plugin manifests, requests, replies and port messages | `internal/plugin` | Plugin host; every plugin, in process and over stdio with the TypeScript SDK ([Plugins](plugins.md#the-contract)) |
| A plugin's page state, page call parameters and results | Page contract types of `internal/plugins/<name>` | Plugin; its page package, through generated TypeScript |
| Machine-manager socket and Cloud image manifest | `internal/machinewire` | Backend; machine manager; `tools/release` writes the image manifest |
| JSON stored in control and conversation databases | The package owning the data, such as `internal/core` for blocks | Backend |

Packages that also own behavior keep their contract declarations separate
from that behavior. Vendor APIs are not Demi contracts: each provider
package declares only the parts it reads ([Providers](../providers/providers.md)).

### Types and markers

Objects are plain structs with explicit `json` tags. JSON field names are
camelCase. A union is a sealed interface with an unexported marker method;
each variant implements it on a pointer receiver. The union carries
`//sumtype:decl`, and `go-check-sumtype -default-signifies-exhaustive=false`
checks every type switch over it: a default case does not hide a missing
variant. Generated validation accepts only declared pointer variants;
an unknown implementation or a typed-nil pointer is invalid.

Markers are line comments on the type or field they constrain. This table
defines the production vocabulary, extending the prototype's smaller set.
Unknown markers, conflicting markers, unsupported types and invalid arguments
fail generation with the type and field location. Bounds are inclusive.
Type constraints apply at every use; field constraints may narrow them,
never relax them.

| Marker after `// +demi:` | Placement and meaning |
|---|---|
| `root direction=receive output=protocol` | Type; a contract root. Without `output`, this is a Go-only boundary and `direction` may be omitted. `direction` is `receive` or `send`, from the web app's perspective; `output` is `protocol`, `web` or `plugin-<name>`. Every root and every type a boundary decodes gets `Decode<Type>`. |
| `schema` | Type; generate `<Type>JSONSchema() json.RawMessage` for a command input or result from the same checked contract model. |
| `union tag=type` | Interface with exactly one unexported method, a parameterless and resultless seal selected independently of method order; exported methods are allowed and implemented by every variant. Internally tagged union. The tag may instead be `op`, `status`, `kind` or `ok`, as the wire requires. `variant true` and `variant false` use JSON boolean tags, never strings. |
| `union untagged` | Go-only interface; decode the first strict struct variant that decodes, in declaration order, in JSON and MessagePack. Encode the variant's own object. TypeScript reachability is an error. |
| `variant text` or `variant Block text` | Struct; pointer variant with the given wire tag value, naming its union when the package has several. The encoder adds the tag; no tag field is declared. A variant can implement several unions with the same wire representation. Untagged variants use `variant` without a tag and implement their sealing method. |
| `enum value1 value2` | Named string type; closed set of wire strings, including singleton literals. |
| `nullable` | Field; required key whose value may be null. |
| `tolerant` | Struct; ignore unknown keys in Go. Every other object is strict. |
| `length chars min=1 max=64` | String type or field; Unicode scalar count. Arrays omit `chars` and count elements. Either bound may be omitted. |
| `pattern ^[0-9a-f]{64}$` | String type or field; shared regex subset below. The remainder of the line is the pattern. |
| `range min=0 max=9007199254740991` | Numeric type or field; either bound may be omitted. Integer kind comes from the Go type; web integers must fit the safe range. |
| `check validateName` | Type; call the named `func(Type) error` after structural checks, in Go only, without IO or mutation. |
| `id` or `id pattern=<regexp>` | Named string type; emit `Parse<Type>(string) (<Type>, error)` with its length, pattern and checks. |
| `timestamp` | Named string type for canonical JSON time, or the runner's milliseconds encoded as a MessagePack timestamp. |
| `base64` | Byte-slice type or field; base64 in JSON, bin in MessagePack. |
| `msgpack` | Type; generate MessagePack codecs for it and everything it reaches: JSON field names in declaration order, a union's tag first, compact integers, string-keyed maps sorted. |
| `msgpack tuple` | Union; the kept-output form: a one-entry map from variant name to its fields as a declaration-order tuple, a single field as the value itself. |
| `format email`, `format http-url`, `format trimmed` | Named string type; email, HTTP(S) URL or trimmed-text behavior in the supported subset below. |
| `table` | Package-level variable of a slice of structs or scalars; a constant table emitted with its generated lookups into `tables.ts`, such as the file-type table. Its TypeScript name is SCREAMING_SNAKE case (`PreviewTypes` becomes `PREVIEW_TYPES`). |

A contract type is a type reached from a `root` marker, including the root
itself. Unreached types are left alone, even in a package that also holds
runtime types. `schema` and `msgpack` select generated capabilities; they do
not make an unreached type a boundary. Roots are markers on types, never a
second registry in the generator.
Named generic instantiations can be roots. Constant tables and lookups come
from their owning Go declarations, without parallel TypeScript tables.

### Encoding conventions

- Optional fields are pointers with `omitempty`: absent is nil, a present
  empty value is kept, and explicit null is refused. For example:
  `Snippet *string` with tag `json:"snippet,omitempty"`. Required fields
  never acquire a default from Go's zero value.
- A non-pointer map or slice with `omitempty` means absent is empty. A
  present empty value is accepted, null is refused, and empty values are
  omitted when encoding JSON and MessagePack. This preserves manifest and
  package digests.
- Nullable fields are pointers without `omitempty`, with the nullable
  marker: their key is always written, nil writes null, and absence is
  refused. A nullable union uses a nil interface, never a pointer to an
  interface. A nullable array is `*[]T`: nil means null and a pointer to an
  empty non-nil slice means `[]`. Ordinary arrays and maps require non-nil
  empty values to encode `[]` and `{}` rather than null.
- Unions are tagged wherever the web app sees the type. Go-only boundaries
  can use untagged unions with strict struct variants. Transcript patches
  use `op`, nested outcomes use `status`, and views use `kind` where their contract says so.
  A tagged variant's standalone JSON encoding includes its tag. If a payload is
  also a separate root, declare it once and compose it into the tagged
  envelope rather than changing its encoding by call site.
- Bytes are base64 strings in JSON. Empty bytes use a non-nil empty slice,
  never null.
- Timestamps are UTC RFC 3339 strings with exactly three fractional digits,
  such as `2026-09-21T14:13:20.000Z`. No offset, omitted fraction, extra
  precision or other spelling is accepted, even for the same instant.
  Calendar validity and exact spelling are checked. Their text orders as
  their times do. This deliberately narrows the previous parser's accepted
  input; every stored timestamp already uses this spelling.
- Integers travel as integers, never through floating point. Go widths and
  signedness are checked before conversion. Web-visible integers must fit
  `[-9007199254740991, 9007199254740991]`; unsigned types also have their
  inherent minimum of zero. Generation fails if the type's range is wider
  and lacks a sufficient bound. Generated Go validation enforces it even
  for types only the backend sends. MessagePack-only integers retain their
  full Go integer range.

Identifiers are named Go strings with validating constructors. Go cannot
prevent `core.BlockID("")`: validity is a boundary guarantee, not a guarantee
of every value constructible in Go. Decoders validate identifiers; internal
code creating one calls `core.ParseBlockID`. Parent validation checks them
too. This is a known difference from the previous private identifier
representation.

[Storage](../backend/storage.md) owns column types, database times, digests,
sealed credentials and password hashes.

### Generated code and wire encodings

`tools/contractgen` reads Go syntax and types with
`golang.org/x/tools/go/packages` and `go/types`. Reading types instead of a
schema file keeps imported types, integer widths, pointer presence and
custom rules in the one definition the Go compiler checks. Declaration
loading must work without existing generated files: source types and rule
signatures cannot depend on generated method bodies.

Each contract package commits one generated file, `contract_gen.go`, beside
its declarations. It contains `Decode<Type>([]byte) (<Type>, error)` for
JSON, `Validate() error` methods, `MarshalJSON` and `UnmarshalJSON` methods on
objects and variants, and a `<Union>JSON` holder with a `Value <Union>` field
and both JSON methods, because an interface cannot own methods. Nested unions
use generated field dispatch, and union validation is
`Validate<Union>(<Union>) error`. JSON methods and holders run the same checks
as direct decoding, including tags; encoding validates the value before
writing it. Errors carry wire field paths and array indices, with wrapped
causes.

Types reached from a `msgpack` root get `MarshalMsgpack` and `UnmarshalMsgpack`
methods, `Decode<Type>Msgpack`, and, for a union, `Encode<Union>Msgpack`.
Zod source is emitted separately to the TypeScript destinations below and is
not committed.

The runner uses `github.com/vmihailenco/msgpack/v5` with
`UseCompactInts(true)`, declaration-order struct maps, tag first, and sorted
string-map keys. `SetSortMapKeys(true)` does not cover every map type;
generated map encoders sort all supported string maps, including nullable
environment values. Empty bytes encode as empty bin. Strings and arrays
in place of bin are refused. Timestamps use extension -1 and the shortest
32/64/96-bit payload. The decoder checks extension kind and length,
nanoseconds below one billion, and overflow converting seconds to signed
64-bit milliseconds before the library can normalize invalid values. The
runner timestamp retains its wire rule of discarding sub-millisecond
nanoseconds; the exact JSON spelling rule applies to JSON timestamps.
Kept-output records use their external tag and tuple representation. Decoders distinguish absent keys from nil, reject duplicate
keys, unknown tags, invalid scalar kinds, overflow and trailing data, and
enforce the same presence and bounds rules as JSON.

The machine-manager Unix socket carries one JSON document followed by a newline.
Typed encoders preserve declared member order and disable HTML escaping.
Successful responses are decoded using the outstanding request's operation,
not by guessing from the result shape. Request envelopes and parameters
tolerate unknown keys; nested boot and image types retain their own
strictness. Invalid device identities remain operation errors as specified
by that protocol.

RFC 8785 digests use `github.com/gowebpki/jcs` and `crypto/sha256`. JCS owns
number formatting, UTF-16 property ordering and escaping; ordinary sorted
JSON is insufficient. Its number model is binary64, unlike the runner's
lossless integers. Manifest hashing covers `roots` and `packages`, excluding
`hash`; package keys hash canonical descriptors. Golden corpora with every
message kind pin runner and machine-manager bytes at both ends, kept
records, and manifest/package digests.

### Generated JSON Schema

A command declaration calls `<Type>JSONSchema()` on a type marked
`+demi:schema`. The function returns fresh JSON bytes from `contract_gen.go`;
callers cannot mutate another caller's schema. Generation uses draft 2020-12
keywords without a `$schema` meta-schema declaration. Subschemas are inline
except cycles: a reference back to the root uses `$ref: "#"`, and other
recursive types use `$defs` and `$ref`, as schemars does even with inlining
enabled.
The declaration's input-subset check still decides whether a schema has a
command-line form; schema generation also supports richer result objects.

JSON tags become `properties` and required fields become `required`.
Optional properties omit `required` and never add null. `nullable` adds a
null alternative with `anyOf`. Objects use `additionalProperties: false`
unless `tolerant`; string-keyed maps use their value schema there. Arrays
use `items`. `union` produces `oneOf` with each `variant`'s tag as `const`.
`enum` produces `enum`, `pattern` (including an `id` pattern) produces
`pattern`, `length` produces `minLength`/`maxLength` for strings and
`minItems`/`maxItems` for arrays, and `range` produces inclusive
`minimum`/`maximum`. Numeric keywords match schemars: `int`/`uint` map
Go's machine-sized integers to Rust's `isize`/`usize`, fixed widths use
`int8` through `uint64`, and floats use `float`/`double`. Unsigned integers
have minimum zero; 8- and 16-bit integers also carry their representation
bounds. Wider integers and floats acquire no extra limits from the generator.
Named-type and field constraints both apply. `timestamp` emits
`type: "string", format: "date-time"`. Opaque JSON emits `true`.
Root direction and MessagePack markers do not change the JSON representation.

A root's `title` is its type name. A contract type's and field's Go doc
comment is copied verbatim from its Rust doc comment and supplies its
`description`, including paragraph and line breaks. Generator directives
are excluded. An explicit field comment replaces the named type's description
at that property; named subschemas retain descriptions but acquire no title.
The lint requirement that an exported comment start with its name is waived
for contract packages. Declaration builders may override a property description
(for example, a browser leaf adds its default deadline to `timeout`);
`internal/declare` and `internal/host` own that text, not the type generator.
No defaults are inferred.

Generation fails with the declaration and field path for reachable `check`,
normalized string `format`, `base64` and byte-slice rules. The built-in command
schemas use none of those rules; new uses need an explicit schema mapping
rather than silently dropping their behavior.
It also rejects the unsupported shapes and invalid markers described above.
`schema` takes no arguments and is only a type marker. JSON Schema validates
parsed values; the generated decoder additionally rejects duplicate keys,
invalid Unicode and invalid numeric token spellings at the JSON boundary.
Numeric `format` annotations do not enforce a Go width, and `date-time` does
not enforce canonical UTC milliseconds: decoders retain those checks, just
as the Rust value types have checks beyond their schemas.

## Validation at entry

When a runner sends `job_exit`, the backend passes its MessagePack bytes to
`runnerwire.DecodeOutbound`. An unknown type, a missing field or an exit
code outside its integer type fails there. Generated validation checks
bounds and custom rules. Connection code receives a typed message or a
field-path error; malformed input closes the connection. Nothing before
the decoder looks inside, and nothing after it repeats the checks.

Every value from outside a process follows the same rule:

- Each boundary uses a generated decoder per message family. Closed sets
  use enums or unions, identifiers use validated named types, and integers
  use integer types.
- JSON decoders build on stable `encoding/json` through `internal/contract`.
  They check UTF-8 before parsing and reject unpaired escaped surrogates
  instead of letting the library replace them. They reject duplicate keys
  at every depth (including opaque JSON and ignored fields), trailing data,
  unknown fields on strict objects, missing required keys and illegal nulls.
  Field names match exactly, without the library's case-insensitive fallback. Tags, scalar kinds, bounds, lengths and
  patterns are checked before custom rules. Presence is checked separately
  from value decoding; `DisallowUnknownFields` alone is insufficient.
  `encoding/json/v2` is experimental in Go 1.27; adopting it once stable is
  a regeneration, with unchanged contract behavior.
- Cross-field rules use the type's shape or named custom Go functions.
  Custom errors are wrapped at the containing field's path.
- Opaque fields, such as tool input, remain JSON values until their owner
  validates them; text explicitly declared as raw input may contain invalid
  JSON. Neither is asserted onto an unchecked contract type.
- Each end validates what it receives. Corrupt data is refused, never
  repaired or defaulted. Only explicitly declared input transformations
  (trimmed text and email canonicalization below) change valid input.
- Fixed contracts use generated checks, never a JSON round trip or JSON
  Schema validation. `github.com/santhosh-tekuri/jsonschema/v6` is used only
  where the schema is data: command arguments checked at both ends against
  the manifest schema with the same failure wording
  ([Parse input and render help](../execution/commands.md#parse-input-and-render-help)),
  and page call parameters checked against the plugin's declared schema by
  the host before the plugin sees them.

String length counts Unicode scalar values everywhere, including command
inputs. Go uses `utf8.RuneCountInString` after Unicode validation, Zod 4
counts code points, and JSON Schema counts characters. JavaScript's
`length` counts UTF-16 code units instead: `😀` is one scalar value and two
code units, so 255 of them fit a 255-character file-name limit
([Commands](../execution/commands.md)). Truncation and token estimates have
their own units ([Token estimates](../agent/compaction.md#token-estimates)).

These are the points where values enter, and what a failure does:

| Where a value enters | Decoded by | When it fails |
|---|---|---|
| A web app request body or query | The edge's body and query extractors, through `webapi.Decode<Type>` | 400 `invalid_body` or `invalid_query`, naming the field and the reason ([Web API](../product/web-api.md)) |
| A frame on the conversation WebSocket | `framewire.DecodeClientFrame` | An `error` frame with code `invalid_frame`, before any state changes; a message that is not JSON closes the socket ([Frame protocol](../agent/runtime.md#frame-protocol)) |
| A frame or REST response the web app receives | The generated schemas, in `conversation-client` and `web` | `conversation-client` drops the connection and reports the field path; `web` validates a response before applying it to state |
| A runner message, at either end | `runnerwire.DecodeInbound` or `runnerwire.DecodeOutbound` | The connection closes ([Runner](../execution/runner.md)) |
| Invocation metadata and records between a runner and a command program | `commandwire.Decode<Type>` | [Validation and flow control](../execution/native-runtime.md#validation-and-flow-control) |
| A command's arguments | The declaration's JSON Schema, at the dispatcher and again in a native handler before work | One usage error that names every field that failed |
| A plugin's page call parameters | The method's JSON Schema from the plugin's manifest, at the plugin host; the plugin uses its generated `Decode<Type>` | 400 `invalid_body`, naming the field ([Plugin calls](../product/web-api.md#plugin-calls)) |
| A plugin's page state or call result, at the page | The plugin package's generated schemas | The plugin's client reports the field path and keeps the state it held |
| A machine-manager request or response | `machinewire.DecodeRequest` or the response decoder for the outstanding operation | A malformed line or an unknown operation drops the connection; an invalid device id is that operation's error ([Managed Cloud hosts](../cloud/managed-hosts.md)) |
| The managed boot file | `runnerwire.DecodeManagedBoot` | The runner fails; it never falls back to pairing ([Runner](../execution/runner.md#managed-guests-and-verification)) |
| A capture extension event | `browserop.Decode<Type>` | The extension connection fails, and the failure is logged ([Live view](../browser/live-view.md)) |
| A row or JSON column read from a database | `internal/backend/database`, through the owning contract's generated decoder | The restore stops; nothing is repaired or defaulted ([Storage](../backend/storage.md)) |
| A sealed credential document | The vault, through its generated document decoder | The error names the field path and the kind of failure, never the value ([Providers](../providers/providers.md)) |
| Configuration from arguments and the environment | Each program's configuration, parsed with the `flag` package and its environment variables at startup | The program does not start, and the error names the variable |
| A tool call's input from the model | The tool | The model receives the tool's error ([Tools](../agent/runtime.md#tools)) |
| A vendor API response | The provider package's two-step decode | An unknown `type` is skipped; a known one with a malformed payload is a protocol failure that is never retried automatically ([Providers](../providers/providers.md)) |

## Generated TypeScript

```text
Go types + JSON tags + markers
   | tools/contractgen: one checked type model
   +-> contract_gen.go in each owning package (committed)
   +-> Zod source + z.infer types (uncommitted)
       -> packages/protocol/src/generated/
       -> packages/web/src/api/generated/
       -> packages/plugin-<name>/src/generated/
```

- **Generation.** `bun run contracts` runs `go generate ./...` and then
  `go run ./tools/contractgen -ts`. Each contract package has a generation
  directive invoking the repository's generator for that package. Scripts that need TypeScript (`typecheck`,
  `typecheck:web`, `test`, `web:dev`, `web:gallery` and `web:build`) run
  generation first. Ordinary Go builds use committed generated Go and need
  no JavaScript tooling; neither builds nor generation require JSON v2.
- **Roots and destinations.** Root markers declare direction from the web
  app's perspective. Every referenced type is emitted transitively; receive
  and send reachability propagate through nested and cross-package types.
  `@demicodes/protocol` exports core types, socket frames and page-visible
  browser live view types through `generated/contracts.ts`, and tables
  through `generated/tables.ts`. The REST roots produce
  `packages/web/src/api/generated/web-api.ts`. Plugin roots are page state
  (received), method parameters (sent) and results (received). REST and
  plugin outputs import shared schemas from `@demicodes/protocol` rather
  than declaring them again. A new body type gets a root marker and direction.
  Go initialisms become capitalized words: `WireAPI` emits `WireApi` and
  `wireApiSchema`, `BlockID` emits `BlockId`, and `HTTPFailureRecord` emits
  `HttpFailureRecord`. Exports match the Rust emitter for the same roots;
  factored variant and scalar schemas remain module-private unless rooted.
- **One constraint definition.** The same marker drives the Go check and
  emitted schema. Tagged interfaces become discriminated unions; optional
  and nullable fields keep their distinct presence rules at both ends.
- **Supported subset.** Objects; string- or boolean-tagged unions including
  a struct payload extended with its tag; string enums and literals; arrays and records;
  optional fields (`.optional()`, refusing null); nullable fields
  (`.nullable()`, requiring presence); string lengths and patterns; integer
  and number bounds, with web integers in the safe range; timestamps with
  the one UTC spelling (`z.iso.datetime({ precision: 3 })`); email addresses;
  HTTP and HTTPS URLs (`z.url` restricted to those protocols, `webapi.EndpointURL`);
  text explicitly trimmed on arrival before bounds are checked
  (`z.string().trim()`, `webapi.Trimmed`); JSON values (`z.json()`);
  flattened plain embedded structs (merged properties, rejecting name
  collisions); one named instantiation of a generic root; recursion through
  named references, emitted as getters on referring object properties so
  Zod can type them recursively; strict and tolerant objects; and constant
  tables with generated lookups (file types, model-readable file types and
  live view frame constants). Anything else fails generation with the type
  and location. Trim and email rules must match at both ends; a custom
  Go-only rule cannot substitute for a supported shared constraint.
- **Regex subset.** Patterns must not use constructs Go's `regexp` and a
  browser could interpret differently: `\d`, `\w`, `\s`, `\b`, `.`, groups
  other than `(?:`, Unicode properties or language-specific character-class
  syntax. Browser patterns use the `u` flag to match by code point, as Go
  does. The generator rejects patterns outside this common subset rather
  than silently translating them.
- **Rules only Go checks.** Custom Go functions are not emitted into Zod.
  The receiving Go end checks them, including completion identity and
  cross-field relationships.
- **Strict and tolerant objects.** Each end judges what it receives. Go
  objects received from the web are strict, including types both ends
  receive: settings arrive in preference patches and leave in conversation
  lists; model selections travel in blocks and are restored from storage.
  The web's schemas for everything it receives are tolerant, including
  nested blocks and selections, so a page left open across a deploy ignores
  newly added fields. Only schemas the web never receives are strict, such
  as client frames; generation refuses these if their Go object is tolerant.
  The machine-manager protocol's explicit tolerance remains independent of
  this web-facing rule.
- **Tolerant values where the web app receives.** Receive-only schemas may
  accept more than Go can hold, because the backend never sends the
  difference: failure-map keys may be empty even when a block ID cannot,
  and email text may carry capitals that `EmailAddress` lowercases. Where
  the web sends a value, its schema refuses whatever Go refuses: trimmed
  names are trimmed before length checks, blank names fail, endpoints must
  use HTTP or HTTPS, and email addresses must have no surrounding whitespace
  and the form Go checks. Every address accepted by the sending schema is
  accepted by Go. Types used in both directions preserve these sending
  value constraints while tolerating unknown object fields on receipt.
- **Checked on arrival.** `conversation-client` validates every received
  frame; `web` validates every REST response before applying it to state.
- **One patch applier.** Transcript patches have one applier,
  `applyTranscriptPatches` in `conversation-client`. The agent's Go tests
  write patch sequences and the snapshots each must produce; client tests
  apply them. There is no second applier in Go.

## The TypeScript boundary

Demi will have a TypeScript SDK for [plugins](plugins.md), and none is built
yet. The design keeps it possible without embedding the Go runtime: a
TypeScript program is a client or a peer of serializable protocols, never a
host of the Go code. Two such protocols exist:

- **The agent frame protocol.** `ConversationClient` drives a session through
  it, as the web app does.
- **The plugin contract.** A plugin receives requests and acts through a port,
  and every request, reply and port operation is a message
  ([The contract](plugins.md#the-contract)). A plugin process written with the
  SDK would exchange the same messages as the plugins the backend links, and
  an `rpc` command leaf would reach it as a command request.

The constraint that keeps this possible: everything a plugin, or an `rpc`
handler, receives is expressible as messages. That covers its arguments,
byte IO, working directory, environment, cancellation, command storage, its
values and blobs, and the Host directories it keeps. The command system
dispatches through `RPCHandler` with a serializable `RPCInvocation` and an
`RPCPort` whose every operation is a message, and the plugin port extends the
same rule to the plugin's other requests. A plugin never receives a live
object, such as a Host handle or a callback into the backend. This is why
command storage and a plugin's values change by versioned compare-and-set
rather than by a callback inside a transaction
([Command state history](../agent/command-state-history.md)), and why a
plugin names the directories a Host must hold instead of writing to a Host.

## Logic the web app and backend share

Logic that both the web app and the backend need gets one owner, chosen case
by case:

| Logic | Owner | How |
|---|---|---|
| The file-type table: which files the product previews, by extension, and which the page shows in place | `internal/core` | The page must choose a viewer before any byte arrives ([Choosing a view](../product/file-previews.md#choosing-a-view)), so the table and its lookups are emitted into `@demicodes/protocol`, and the backend serves files by the same definition |
| Whether an upload is text, and its short opening snippet | Backend | The upload response carries the snippet the composer's tile shows |
| The media type a model receives for an upload | Backend | The upload response carries the sniffed media type; the message editor uploads files the way the main composer does |
| Whether a message can be edited | The data model | `User` is the only editable block type; hidden inputs are `Context`, `Wakeup` and `AgentMessage` blocks |
| The summary text of a queued message | Web app | The queue carries each message's content, and `web-ui` derives the summary |
| Completion message ids | Backend | The rule that ties an id to its sender and round is a custom Go check, which is not emitted; the web app receives ids as data |
