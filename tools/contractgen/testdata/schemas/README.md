# Command schema expectations

Neither `crates/runner-protocol/tests/fixtures/manifest.json` nor
`crates/command-declarations/tests/fixtures/cli.json` carries schemas for the
real file or browser types. Both contain a synthetic `fixture read` command
(with a count default and other fields absent from real `ReadArgs`). The
expectations here are therefore hand-derived from the Rust declarations,
as the work-package brief permits, not claimed to be captured Rust output.

The comparison pins validation keywords. Rust's schemars also emits titles,
doc-comment descriptions and numeric format annotations; the Go marker
vocabulary has no such annotations, and this generator omits them.

| Expectation | Rust source and derivation |
|---|---|
| `ReadArgs` | `command-package-file-protocol/src/lib.rs`: required string `path`, `deny_unknown_fields`. |
| `CreateArgs` | Same source: required strings `path` and `content`, strict object. Empty content is valid. |
| `EditArgs` | Same source: required `path`, `old`, `new`; `old` has length minimum 1; optional `usize` occurrence and context have minimum 1. The target platforms are 64-bit, so their representation maximum is 18446744073709551615. Optional properties never accept null. |
| `PatchArgs` | Same source: required string `patch`, strict object, no length bound. |
| `OpenInput` | `command-package-browser-protocol/src/browser/operations.rs`: required URL string length 1–4096, optional Load enum, optional timeout 1–300000 contributed by `input!`. `browser/mod.rs` defines the constants and the three Load strings. |
| `NavigationResult` | `browser/operations.rs`: required TabId and URL, optional non-null title. `browser/mod.rs` defines TabId's exact `^t[1-9][0-9]{0,14}$` pattern. |
| `CloseResult` | Same sources: required `closed` TabId, strict object. |

The Go `range` and `length` markers deliberately carry Rust's garde rules as
well as its representation: the schema must agree with the decoder. These
hand-derived expectations include those rules even where a Rust derive may
not translate a garde annotation into a schemars keyword. Numeric width
maxima are explicit in Go schemas; Rust may leave a width in a non-asserting
`format` annotation instead. These are validation-strengthening differences,
not byte-for-byte Rust schema claims.

`ExampleArgs` ports
`command-declarations/tests/commands.rs::declared_argument_types_generate_schemas_inside_the_subset`:
required path and tags, count maximum 9, fast/slow enum, no meta-schema and
no null alternatives. `Outcome`, `Collection` and `Constraints` are additional generator
fixtures for tagged unions, nullable fields, tolerant objects, arrays and
nullable map values, and intersected type/field constraints; they do not claim to represent Rust commands.

`TestCommandSchemas` compiles every expectation using draft 2020-12 and runs
valid and invalid values through both the generated schema and decoder. Its
EditArgs scenarios port
`command-package-file-protocol/tests/protocol.rs::file_arguments_refuse_empty_old_text_and_zero_positions`.
The file operation-dispatch test is outside this generator's responsibility.
Other command-declaration tests concern argv parsing, help and registration,
not schema generation. Browser operation dispatch, runtime deadlines, capture,
live-view and release tests likewise belong to their protocol work packages.
