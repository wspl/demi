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

## Codex Responses ingress

`provider-codex/response-schemas.ts` owns the consumed Responses event union.
`sse.ts` and `transport.ts` validate each decoded event before `responses.ts` maps
it. WebSocket envelopes and `response.done` are explicit transport mappings into
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
SSE readers cancel and release their lock on completion, parsing failure or early
consumer return. WebSocket listeners and timers are removed when consumption ends.

## Claude Code ingress

`provider-claude-code/transport.ts` decodes JSONL into unknown values. Its syntax
errors omit line contents. `output-schemas.ts` validates each received message
through `readClaudeMessage`, including SDK initialization and continuation reads
from injected transports. `output.ts` only maps validated messages.

Assistant content and streamed text, thinking, signatures and redacted data require
their string fields; empty strings are valid. Tool blocks require string IDs and
names. Missing tool input is invalid, and explicit null input remains null for the
agent's tool schema to reject. MCP requests require a string or integer request ID;
notifications can omit it and receive no response. SDK envelopes require the outer
request ID and the `main` server. Missing MCP tool names are protocol failures.
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
tests. Run selected suites with `bun test --conditions development <test paths>`;
runner integration suites additionally require their documented native test setup.
Run `bun run typecheck`; changes to browser source also require browser typechecks
and verification of both product and gallery usage.
