# Data Contracts

This document defines the required data-boundary design. Package ownership and
allowed dependencies are defined in [Package Boundaries](package-boundaries.md).
Design acceptance is separate from runtime verification: each implementation
checkpoint must demonstrate its input and error behavior with independent tests.
Historical plans and existing implementations do not create exceptions.

## Responsibilities

| Input | Responsible module | Required result |
| --- | --- | --- |
| Model tool arguments | `agent/tools.ts` | Validate the complete tool input before resolving an environment or executing a command. Generate the model declaration from the same schema. |
| Command argv | `shell/command.ts` | Group tokens and convert CLI strings, then validate with the command's input schema. |
| RPC command arguments | `command-loader/loader/rpc.ts` | Validate decoded values against the declaring command's schema without argv conversion. |
| Command manifest | `command-loader/manifest/` | Validate the manifest and reject unsupported schema semantics before dispatch. Supported schemas must survive serialization with identical behavior. |
| Provider HTTP, SSE, WebSocket and JSONL data | The concrete provider's transport/schema module | Decode into `unknown`, validate consumed fields, then map typed events into provider events. |
| Agent and runner frames | The receiving protocol boundary | Validate the inbound direction after decoding. Both endpoints validate received serialized data, including peers written by Demi. |
| Persisted domain state | The domain storage reader | Decode and validate before returning a domain object. A generic codec does not validate domain state. |
| Product HTTP data | Backend route ingress and browser `web/api` ingress | Validate incoming bodies and responses. Share wire contracts in a dependency permitted for both consumers; never import backend code into the browser. |
| Browser persisted preferences and drafts | The owning browser state module | Validate the stored representation and apply its explicitly documented failure policy. |

Validation belongs at the first layer that knows the consumed contract. Downstream
functions accept the validated type; they do not repeat partial validation. Wire,
persisted and display representations that differ have distinct types and explicit
mappers. An inline media source and a stored blob reference are not interchangeable.

`core` owns dependency-free value sets and their derived types. Boundary packages
build enum schemas from those value sets. `utils` owns decoding, byte framing and
generic error inspection; it has no domain schemas or Zod dependency. Provider wire
schemas belong to the concrete provider; genuinely shared provider contracts belong
to `provider`. A new shared package or entry requires a registry and dependency-graph
update before consumers use it.

## One contract

Define structured runtime validation with Zod and derive TypeScript types with
`z.infer`. Generate external JSON Schema from that definition when the schema is
serializable. Do not maintain a JSON Schema declaration and a hand-written field
parser for the same input. A `z.ZodType<T>` annotation verifies assignability, not
that a custom predicate actually checks every required field of `T`.

Raw JSON is `unknown`. A cast, an object guard, or a check of `type` alone does not
validate its structure. After a discriminated union has been validated, direct
tag comparisons are ordinary type narrowing and should remain simple.

## Boundary examples

- `JSON.parse(text)` only decodes JSON. For a saved credential entry, the provider
  storage reader must parse the decoded value with its metadata schema before it
  can return a credential. `JSON.parse(text) as Credential` accepts wrong shapes.
- After an agent frame passes its direction-specific union schema, checking
  `frame.type === 'send'` narrows a validated value. Testing only an unknown
  object's `type` and claiming the whole frame type leaves its content unchecked.
- CLI `--enabled false` decodes to boolean `false`, then uses the command schema.
  RPC `{enabled: "false"}` is rejected because RPC already carries typed values.
- A provider's known text delta may contain an empty string and unrelated new
  fields. A text delta whose `text` is an object is a protocol error; the mapper
  must not turn it into an empty string. Unknown event policy is defined separately
  for each provider below.
- A product transcript image can contain a blob `ref`. Its display schema accepts
  that shape; the inline provider schema rejects it. Backend media resolution
  reads the blob before passing inline bytes to inference. An assertion cannot
  replace that read or make a missing blob valid.

## Missing and invalid values

| Case | Policy |
| --- | --- |
| Missing optional field | Accept absence; apply a default only if the contract declares one. |
| Explicit `null` | Accept only where the contract permits `null`; do not convert it to absence or wrap it in an array. |
| Wrong field type | Reject; never convert it to a default, zero, an empty collection, or a fabricated identifier. |
| Unknown tool or command input key | Reject before execution. |
| Extra fields in a supported provider event | Permit unrelated extensions while validating every field Demi consumes. |
| Unknown provider event | Follow that provider's explicit event policy. Never treat a malformed supported event as an ignorable unknown event. Terminal, error and tool-call events require explicit handling. |
| Missing durable file | Return the storage API's absent result only for `ENOENT`. |
| Corrupt durable data or IO failure | Report a distinguishable error. Do not present it as absent credentials, an empty conversation, or an empty draft. |
| Rebuildable catalog cache | Discard the invalid cache as a whole and fetch again; expose stale/fallback state when serving cached or fallback results. |
| Disposable visual preference | The owning module may discard the complete invalid record and use its documented visual default. |

Schema errors must identify the field and operation without logging credential or
payload bodies. Error inspection through shared helpers is allowed; catching every
error and returning an absent result is not.

## Commands and tools

Tool inputs use strict objects. `shell_write.stdin` contains at least one character;
whitespace and newlines are valid data. Tool delays are integer milliseconds from
1 through 600000 inclusive. Reject fractional values instead of silently rounding.
Tool descriptions belong to the model-facing contract and do not enter shell input.

Only argv decoding converts string numbers and the case-sensitive strings `true`
and `false`. Empty numeric strings are invalid. Repeated array options convert each
element. Typed RPC inputs preserve their types and nullable values. CLI token
grouping, switches, stdin and positional routing remain shell responsibilities.

Command schema support must be explicit. A remote manifest cannot carry arbitrary
JavaScript refinements, preprocessing or transformations. Reject unsupported
behavior at registration/build with the command and field name; do not silently
drop it during JSON Schema conversion. Help, original trees, reconstructed trees,
runtime modules and RPC handlers must agree on the supported input semantics.

## Shared provider SSE framing

`provider/sse.ts` reads complete event frames for the OpenAI, Anthropic, Google,
Grok Build and Codex adapters. It follows the
[SSE framing rules](https://html.spec.whatwg.org/multipage/server-sent-events.html#event-stream-interpretation):
CR, LF and CRLF delimit lines, one leading ASCII space after the field colon is
removed, and data lines join with a newline. A blank line resets the event name
even without data. Comments and unused fields do not reach provider mappers.
EOF discards an unfinished frame; it does not confirm provider completion.

The reader rejects missing bodies and malformed UTF-8 with payload-free
`invalid_provider_response` errors. Cancellation wakes a pending read and throws
the signal's abort reason. Completion, cancellation, parsing failure and early
consumer return share cancellation, listener removal and reader-lock release.
Concrete adapters own JSON schemas, terminal markers and provider error rules.

## Shared Responses ingress

`provider/responses-wire.ts` owns the consumed Responses event union and usage
projection shared by the Codex and OpenAI API adapters. `provider/responses.ts`
projects their identical content events and tracks delta/full-item deduplication. Codex's `sse.ts` and
`transport.ts` validate events before its diagnostic-aware `responses.ts` mapper.
The OpenAI API mapper validates each SSE payload through the same schema.
Codex WebSocket envelopes and `response.done` are explicit transport mappings into
that union. Invalid JSON, wrong field types and unsupported event tags fail with
`invalid_provider_response`; this error does not trigger an SSE retry. Unrelated
fields are allowed on supported events. Known progress events and hosted tool
items listed in the schema are ignored because Demi does not execute them.

Message content is an array. Reasoning summary/content may be absent but must be
arrays of text parts when present; encrypted content may be null. Function calls
require nonempty item ID, call ID and name, plus a string of arguments. The completed
item supplies the full arguments; argument deltas are validated but do not cause
execution. This follows the complete-item examples in the official
[function calling guide](https://developers.openai.com/api/docs/guides/function-calling)
and [streaming guide](https://developers.openai.com/api/docs/guides/streaming-responses).

Terminal events require a response object. Missing or null usage means unavailable
counts, represented by zero in `TokenUsage`; present counts are nonnegative integers,
and cached input cannot exceed total input. Missing error details produce a generic
failure message; wrong detail types fail validation. Reasoning replay validates
Codex-tagged signatures and omits opaque signatures from other providers.
The OpenAI Responses mapper returns after a completed, failed, incomplete or error
event; `[DONE]` or EOF without one is an invalid response. SSE readers cancel and
release their lock on completion, parsing failure or early consumer return.
WebSocket listeners and timers are removed when consumption ends.

## Shared Chat Completions ingress

`provider/chat-completions-wire.ts` defines the consumed Chat Completions chunk
schema used by OpenAI-compatible and Grok Build endpoints. Its companion
`chat-completions.ts` maps validated chunks. Endpoint selection, authentication,
request building and provider-specific diagnostics remain in concrete adapters.
Responses and Chat Completions are separate wire contracts.

A chunk carries a choices array or an error object. Text and reasoning may be
absent or null; present values are strings. A nonterminal choice requires a delta
object. For compatible endpoints that omit choice indexes, array position is the
explicit choice identity; supplied indexes are nonnegative integers and cannot
collide. Tool deltas always require their integer index. Missing tool metadata is
allowed during streaming, while supplied ID/name values must be nonempty strings
and supplied argument fragments must be strings.

The mapper accumulates tools independently per choice and tool index. Tool ID and
name cannot change mid-stream. At tool completion, ID, name and argument text must
all exist, and duplicate call IDs fail. Argument fragments join before JSON
parsing; malformed model-authored JSON remains a string for tool input validation.
No replacement tool ID or missing argument object is synthesized. A tool_calls
finish emits completed tools; `[DONE]` flushes remaining complete calls and reports
success. EOF without `[DONE]`, malformed chunks and incomplete calls are errors.
Length, content filtering and unsupported finish reasons report errors before tool
execution. No data can be appended to an already finished choice.

Usage is optional or null; unavailable counters project to zero. Present counters
are nonnegative integers, and cached prompt tokens cannot exceed total prompt
tokens. Cached prompt tokens are subtracted from uncached input. Valid extension
fields and compatible reasoning_content deltas remain supported.

## Anthropic Messages ingress

`provider-anthropic-api/response-schemas.ts` validates complete SSE payloads before
`mapAnthropicMessageStream` consumes them. The payload type is required; a supplied
SSE event name must match a known payload type. Blocks require a nonnegative integer
index. Tool blocks require their provider ID, name and object input. Known text,
thinking, signature and JSON deltas require strings. Known fields reject wrong
types; unrelated fields remain available to protocol extensions. Unknown tagged
events, content blocks and deltas are explicitly ignored, following Anthropic's
[streaming extension policy](https://platform.claude.com/docs/en/build-with-claude/streaming).

The mapper tracks active content blocks by index. Duplicate starts, orphan deltas
or stops, mismatched delta kinds and a message stop with open blocks are errors.
Only a stopped tool block emits a tool request. JSON argument fragments accumulate
until that stop; malformed model-authored argument JSON remains a string for the
tool input validator to reject. Missing wire identity is never synthesized.
Only `message_stop` reports success; EOF before that event is an invalid response.

Usage counters are cumulative nonnegative integers. Missing counters preserve the
last observation, including when the entire optional usage object is absent.
An explicit zero replaces the earlier value; null and wrong types are errors.
Anthropic input, cache read and cache creation counts are separate categories.

## Google generateContent ingress

`provider-google/response-schemas.ts` owns the consumed response fields. Candidate,
content and parts containers must have their declared object/array shape. Missing
optional candidates or parts mean no content in that chunk. Text, thought flags
and signatures validate their types before presentation. Extension fields and
unconsumed part variants remain outside the mapper's responsibilities.

Function calls require a valid function name and object arguments when supplied.
Google permits omitted arguments and IDs: these become `{}` and a unique local
call ID respectively. A supplied ID must be a nonempty string. Text and function
call data cannot occupy the same part. Signed call replay and inline input media
retain their existing provider-specific mapping.

The mapper tracks completion per candidate. Prompt blocking or a non-STOP finish
reason produces an error; EOF with an unfinished candidate or without any candidate
does not produce success. Usage counters are nonnegative integers. Prompt tokens
include cached tokens, so the mapper subtracts cache read tokens from input tokens;
visible and thinking output tokens are added. Missing measurements are unavailable
and project to zero. If only cache usage is supplied, uncached input remains zero.
These definitions follow Google's
[response contract](https://ai.google.dev/api/generate-content#v1beta.GenerateContentResponse).

`toGoogleSchema` is an outbound projection into Gemini's supported tool-schema
keywords. It recursively projects properties, items and anyOf branches and omits
unsupported constraints. Tool execution still validates the original tool schema;
this projection is not a validator of tool inputs or provider responses.

## Claude Code ingress

`provider-claude-code/transport.ts` decodes JSONL into unknown values. Its syntax
errors omit line contents. `output-schemas.ts` validates each received message
through `readClaudeMessage`, including SDK initialization and continuation reads
from injected transports. `output.ts` only maps validated messages.

Assistant content and streamed text, thinking, signatures and redacted data require
their string fields; empty strings are valid. Tool blocks require string IDs and
names. Missing tool input is invalid, and explicit null input remains null for the
agent's tool schema to reject. MCP requests require a string or integer request ID;
notifications can omit it and receive no JSON-RPC response. SDK envelopes require
the outer request ID and the `main` server. `provider.ts` acknowledges the outer
SDK request with an empty success response even for an inner notification, so the
CLI can proceed with its next MCP request. Missing MCP tool names are protocol failures.
An initialization error response fails immediately. These failures terminate and
reap the active CLI process before propagating `ProviderDataError` or the explicit
initialization error.

The content and delta fields follow the official
[streaming protocol](https://platform.claude.com/docs/en/build-with-claude/streaming).
Demi consumes complete assistant tool blocks and uses `message_stop` to close a
tool batch; validated `input_json_delta` events do not execute tools. Only the
non-content event tags enumerated in `output-schemas.ts` are ignored. Extra fields
on supported messages are permitted; unsupported tags fail with a protocol error.

Usage is optional, but null or a wrong type is invalid. Counts are nonnegative
integers. The final `iterations` entry supplies response usage; absent or empty
iterations use top-level counts. Missing counts map to zero. The tested legacy
injected transport also supports camel-case counts. Errors in an iteration do not
fall back to the turn total. Error messages and result errors must be strings.

## Verification

Before introducing a helper, compare its actual semantics with the installed
library and existing shared functions. Numeric units, path algorithms, byte codecs,
SSE framing, and partial streaming presentation are not replaced with structural
validation. Presentation tolerance must never become execution validation.

Each changed data path must identify its owner, parse location, error exit and
remaining consumers. Test independently chosen valid and invalid values, including
missing, `null`, empty strings, wrong types, unknown keys and partial events where
applicable. Check serialized round trips separately from the original schema.
Do not derive every expected result from the implementation under test.

Use fake providers, injected fetch/streams, synthetic credentials and temporary
storage. Never invoke real models or read real credential stores in validation
tests. Use the offline commands and native prerequisites in [Testing](testing.md).
The production-source audit includes every workspace and Vue script blocks; its
structural helper comparison and direct JSON-assertion checks supplement runtime
examples and do not prove arbitrary data-flow or semantic equivalence.
Run `bun run typecheck`; changes to browser source also require browser typechecks
and verification of both product and gallery usage.

Provider authentication files, device grants and token responses use each
provider's `auth-schemas.ts`. Their field and missing-value policies are specified
in [global provider credentials](provider-global-credentials.md#6-per-provider-behavior).
The shared provider validation module owns JSON and JWT payload decoding and
value-free schema diagnostics; each provider owns the claims it consumes. JWT
payload parsing is metadata extraction, not signature verification.

Provider creators consume their typed library options, including explicit callback
and auth-store dependencies. Codex and Claude do not expose a separate serialized
config parser. The public Grok `parseGrokBuildProviderConfig` validates serialized
`grokHome`, HTTP(S) `baseUrl` and string-valued `headers` through `config-schema.ts`.
A null/undefined whole config means no configuration. Present fields must satisfy
their schema; unknown keys, null fields and runtime dependencies are rejected.

`core` owns image, video and all-file extension constants and derives their types
from those sets. Agent wire schemas enumerate the same all-file set; provider
model selection uses the image set plus PDF as its default attachment capability.

## Portable JSON and generic storage

`utils/json.ts` owns the portable encoding of `Uint8Array`, `bigint` and `Date`.
Its three marker keys are reserved. A marked object contains exactly the marker
set to `true` and its string payload. Binary payloads use canonical padded base64;
bigints use canonical decimal integers; dates use the exact UTC ISO representation
produced by `Date.toISOString()`. Invalid markers, extra marker fields, invalid dates
and nonfinite numbers fail decoding. Writers reject reserved marker keys in user
objects and nonfinite numbers. Unmarked ordinary JSON remains ordinary data.

`shell/host.ts` defines `HostStore.readJson` as an unknown-value read. File and DB
implementations decode portable JSON and return `null` for absent keys. JSON or
marker corruption and IO failures propagate. The caller owns domain validation;
HostStore cannot infer a domain contract from a key or a TypeScript generic.

`backend/vault/crypto.ts` decrypts plain JSON into `unknown`. The envelope contains
exactly the version, a 12-byte IV, a 16-byte GCM authentication tag and ciphertext,
with canonical base64 fields. Format, authentication, UTF-8 and JSON failures
propagate. `backend/vault/providers.ts` validates the decrypted provider config
using its config schema before returning it. Encryption is not domain validation.

## Agent transport boundaries

`agent/protocol/schemas.ts` and `server-schemas.ts` own the client and server
frame schemas; frame and patch types derive from them. WebSocket and stdio
adapters decode portable JSON into unknown data and validate the inbound
direction. WebSocket frames must be text; stdio uses UTF-8 JSON lines and accepts
CRLF. A custom WebSocket protocol supplies its own validated decoder. Typed
in-process transports already receive domain frames and do not parse them again.

`FrameChannel` preserves delivery order across asynchronous handlers and reports
handler failures through `onError`. Synchronous handlers complete without an
extra microtask per frame. Malformed serialized input reports `invalid_frame`;
socket/stream errors and closure have separate transport error codes. The server
binding replies to invalid client input with an error frame and keeps the
connection usable. Other transport failures detach the binding. The client
disconnects on any transport error, rejects outstanding requests and removes
listeners and timers. Closing a channel releases queued deliveries.

Transcript patch schemas validate integer indices. Patch application checks those
indices against the current transcript and rejects text appends to other block
types. A failed patch leaves the existing transcript intact.

## Transcript representations

`core.Block` describes transcript structure independently of its user and tool
content parameters. Its defaults contain inline model input. `agent/protocol`
owns the corresponding boundary schemas: common block fields are declared once
and combined with the content schemas for each representation.

`agent/store/media-contracts.ts` owns stored and displayed content schemas.
Stored image/video sources contain a blob reference or an existing user URL;
stored documents require a blob reference and filename. User blob sources carry
`type: ref`; tool result blob sources carry `ref` and `mediaType`. Stored blocks
contain no inline media. Displayed blocks accept inline or stored media so the
shared renderer can display product history and local client transcripts.

`agent/store/media.ts` maps inline blocks into stored blocks and rehydrates stored
blocks into inline blocks before inference. The storage reader validates stored
blocks before this mapping. A malformed reference fails validation; a valid
reference whose blob is missing becomes the documented missing-media text.
Blob IO errors propagate. No mapper invents a filename for malformed records.

Agent clients, transcript patches and server frames carry an explicit block type.
Product transcript frames use displayed blocks, while queue, pending-steer and
transient tool-output frames retain their inline content contracts. The backend
externalizes transcript media; the browser validates the same displayed contract
used by the shared renderers. Displayed blocks cannot be passed to model input
without the explicit rehydration mapping.

## Session storage ownership

`agent/store/session-schema.ts` declares the persisted session fields. Backend
`storage/tree-store.ts` validates them and each stored block before returning a
checkpoint. Block indices are contiguous and match `block_count`; invalid node
flags, metadata and block sequences fail the read. Existing command-state schemas
validate version/boundary references. Cold summary reads validate the fields they
consume without rehydrating media.

Harness state remains opaque in the generic store. Each `AgentHarness` supplies
`stateSchema`, which `agent/node/assemble.ts` parses before passing restored state
to commands, host resolution or other hooks. The coding harness owns a strict
empty-object schema. A checkpoint belonging to a different harness is an error;
opening a connection does not delete it. `restoreState` is the separate editing
and fork hook that computes state from retained history.

## Product API contracts

`product-contracts` owns the schemas shared by the backend and browser. Authentication
normalizes email by trimming and lowercasing before the shared email constraint;
stored users contain the resulting address. The UI uses the same acceptance test
and supplies its own field messages. Preferences and manually configured models
use the same schemas for request validation, persistence reads and browser reads.
Null shortcut patches clear an override; missing fields leave it unchanged.

REST response types derive from the shared response schemas. Backend projection
functions declare these types and browser response readers parse them. Storage and
provider domain types remain with their owners; sharing a response does not move
authorization, persistence or provider behavior into the contract package.

Conversation `send` and `steer` frames accept inline user input, upload references
and remote-file references. They reject backend-authored attachment records. The
backend resolves valid product references before delivering a typed agent frame.
No generic frame-union branch bypasses these content constraints.

Editing submits complete replacement content, including retained attachment
records and rehydrated inline media. Product upload/device-reference envelopes
are unsupported in edits: both the browser encoder and server decoder reject
them explicitly. Existing ordinary references remain ordinary references and are
resolved by the harness during edit preparation. This keeps retries byte-stable;
an edit retry does not create another host attachment path.

### Command-owned persisted values

`shell/command.ts` exposes decoded `unknown` through `CommandStorage.readJson`
and the input of `updateJson`. An absent key yields `undefined`; a stored `null`
remains `null`. The command validates its own shape before reading or mutating
it. `coding-agent/todo-command.ts` accepts absence as an empty initial list and
rejects malformed lists, including `null`, before any replacement is committed.

`agent/session` detaches the update result before awaiting storage, validates
portable values through `CommandStateHistory`, and returns the callback's typed
result after committing. A rejected value leaves the revision and stored value
unchanged. `backend/storage/command-state.ts` validates version and boundary
relationships on restoration; control-plane JSON fields retain their named
preference, target, fork and managed-operation schemas.

### Browser presentation and optional host control

`web-ui/agent/block-helpers.ts` parses tool input only for display. Standard tool
rows may display partial object JSON; other tools require a complete object.
Arrays, primitives and malformed input yield an empty display object. This
reader never authorizes execution. Titles trim non-blank strings; tool source
text retains whitespace. Finite-number extraction uses `utils/numberOrNull`.

`web-ui/transport/protocol.ts` owns the optional host control adapter's provider,
model and workspace response schemas. `connectControlClient` validates the
response envelope and each pending method's result. Malformed data closes the
connection and rejects all pending calls. Remote errors reject their matching
call; late or duplicate responses are ignored. Opening and requests have bounded
waits. Close, abort, error and timeout release their listeners and timers. The
embedding host owns and closes the returned control connection.

`AgentWorkspace` remains a supported optional UI host adapter, independent of
the backend product's REST stores. Its local conversation list stores one
ordered array and an active id. Restoration validates the complete record and
its unique-id/reference relationships before creating runtimes. Invalid records
fail initialization and remain stored; no partial list is recovered or written
over it. Absent storage starts a new workspace. Model intent permits empty ids
for an unselected composer; an actual prepare-session call names a selection.

### Browser storage failure policies

| Record | Owner | Invalid or unavailable data |
| --- | --- | --- |
| Authored drafts, pending sends and edits | `web/conversation/drafts.ts` and `store.ts` | Invalid content blocks restoration and remains stored. Failed reads never mark a draft writable. Unavailable storage permits an in-memory session with a visible warning. |
| Scroll positions and virtualized height snapshots | `web-ui/composables/scroll-state.ts` | Reject the entire visual snapshot. A malformed scroll field does not discard the draft containing it. |
| Product local display preferences | `web/state/local.ts` | Reject the entire preference record and use defaults. |
| Gallery style selection | `web-gallery/gallery-state.ts` | Validate every stored axis, then apply the selected preset or valid custom axes. Reject the entire invalid record and use the default appearance. |
| Theme choice | `web-ui/theme/appTheme.ts` | Only `light` and `dark` are stored choices. Invalid or inaccessible storage falls back to the system; write failure keeps the current appearance in memory. |
| Runner config and active manifest | `runner/state.ts` and `manifest-cache.ts` | Only a missing file is absent. Decode, schema and IO failures propagate. |
| Machine image pointer | `machines/machine-image-store.ts` | Only a missing pointer is absent; invalid manifests fail without replacing a generation. |
| Provider catalog cache | `provider/models-dev.ts` and provider catalog modules | Validate before caching; return independent snapshots and explicit stale status after refresh failure. |

Token-count parsing belongs to `web-ui/ui/token-count.ts`: it removes visual
separators, converts K/M units, rounds to whole tokens and rejects unsafe results.
Display precision preserves individual tokens across unit changes. This is UI
conversion, distinct from extracting a finite number from an unknown record.
File-browser paths retain POSIX semantics; Markdown file detection continues to
recognize Windows paths and file URLs. These are separate formatting contracts.

## Process startup and local execution

`backend/startup.ts`, `machines/startup.ts` and `runner/startup.ts` validate
process environment values before constructing services. Unrelated environment
keys are ignored. Present path/name values must not be blank. The backend public URL uses
HTTP(S); the backend instance mode reuses the product contract. A machine socket
requires the guest-facing backend URL. Backend ports are decimal integers from
1 through 65535. Defaults apply only to absent variables.

Runner `DEMI_RUNNER_MANAGED` accepts `0` or `1`; absence means false. Reconnect
milliseconds are decimal integers from 1 through 2147483647. CLI backend URLs
use the same HTTP(S)/WS(S) URL schema as persisted runner configuration. Runner
WebSocket URLs remain valid; pipe URL construction derives the HTTP(S) origin. Library-only runtime
dependencies such as hosts, volume implementations and callbacks remain typed
injection points, separate from environment parsing.

`machines/firecracker/config.ts` owns the inferred Firecracker configuration and
its environment projection. Counts and capacities are positive safe integers;
vCPU count is limited to 255. DNS is a nonempty comma-separated list of IPv4
addresses without empty entries. The subnet must be an aligned IPv4 network of
/30 slots, and the requested slot count must fit. Jailer fields require jailer
mode; uid allocation must fit the unsigned uid range. Partial managed settings
without a Firecracker binary fail. Parsing these values does not execute a VM.

`runner/state.ts` treats only ENOENT as a missing token. Token files contain one
non-whitespace token plus optional surrounding file whitespace. Empty or
malformed content and IO failures propagate without deleting or replacing the
file. Writers validate before writing and retain mode 0600.

`runner-protocol/local.ts` owns local IPC frame lengths, tags and handshake
schemas; `local-contract.json` also generates the native C constants. Receivers
reject unknown tags, oversized frames, truncated input and malformed handshake
fields. The native client separately enforces handshake ordering, output limits
and exit status. These byte and connection-state checks are not command schemas.

`runner/commands/worker-messages.ts` owns structured-clone message schemas for
both worker directions. `execute.ts` validates worker IO/result messages before
performing IO; `worker.ts` validates run/reply envelopes. Arguments remain an
opaque named-value record until command-owned validation. Invalid messages or
send failures terminate execution through the same cleanup as exit and abort:
remove listeners, terminate the worker and release stdin. Completed or failed IO
replies remove their pending request entries.

## Release metadata

`scripts/release-contracts.ts` validates consumed workspace, package and registry
metadata. Package fields not consumed by validation are preserved for publication.
Dependency values are strings; exports use the recursive package.json shape.
Tarball membership and workspace-version consistency remain explicit semantic
checks in `scripts/release.ts`. A malformed registry success response fails;
only HTTP 404 means no published versions.

`runner/runtime/release.ts` validates requested targets with the existing
`runner-protocol/release` enum before building. Generated manifests use the same
release schema as readers. No publication or registry call is needed to test the
metadata schemas. Library capabilities are checked against the installed lockfile
and local behavior tests, rather than inferred from a major-version label.
The workspace declares Zod `^4.5.4` and locks 4.5.4. Command-schema round-trip
tests exercise `toJSONSchema` and `fromJSONSchema` on that installed version;
this is the supported minimum, not a claim about when each API first appeared.

## Optional Grok client identification

`provider-grok-build/headers.ts` validates local `version.json` with a version
string schema. This file supplies optional client-identification metadata only.
Missing, malformed or unreadable metadata selects the built-in client version;
credential and token errors still follow the strict authentication policies.
`clientVersion` and `grokHome` are explicit dependencies. Device login resolves
one version for the complete flow, and tests inject a synthetic version or a
temporary directory rather than reading the user's Grok installation.
