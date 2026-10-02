// Contractgen generates boundary codecs from annotated Go declarations.
//
// Run it in a contract package with:
//
//	//go:generate go run github.com/wspl/demi/tools/contractgen
//
// Or pass package patterns from the repository root. The -check flag compares
// generated files without writing them. The -ts flag discovers root markers
// under ./... by default and writes the frontend's established destinations.
// -ts-dir redirects those destinations for fixture comparisons. Generation
// requires GOFLAGS=-mod=readonly while the Rust vendor directory remains.
//
// Structs use explicit JSON field names and are strict unless marked tolerant.
// Optional fields are pointers with omitzero or omitempty; default-false bools
// may also be optional. A nullable pointer or union instead has +demi:nullable
// and must be present. Nil required arrays and records are invalid; construct
// empty values explicitly. Untagged embedded value structs flatten their
// properties, including validation. A concrete declaration such as
// "type Names Page[Identifier]" instantiates a generic shape and its field rules.
//
// A union interface has +demi:union tag=type and one unexported, parameterless
// sealing method. Its pointer variants have +demi:variant <tag>; when a package
// has multiple unions, supply the sealing method or use
// +demi:variant <Union> <tag>. The generator supplies absent sealing methods.
// Variant JSON includes the tag even when encoded outside its union.
//
// Scalar and field rules are +demi:length chars min=1 max=64 (array lengths omit
// chars), +demi:pattern <regexp>, +demi:range min=1 max=9007199254740991,
// +demi:enum first second, +demi:timestamp and +demi:base64. An identifier uses
// +demi:id (optionally pattern=<regexp>), which generates Parse<Type>. A type's
// +demi:check <func> names a func(Type) error checked only in Go. JSON values use
// encoding/json.RawMessage. Timestamp domain types remain named strings in
// their owning contract package; their marker enforces canonical UTC milliseconds.
//
// Named strings may use +demi:format trimmed or +demi:format email. Decoders
// and Parse constructors trim JavaScript whitespace before length checks;
// email also lowercases and checks the address grammar and 254-character cap.
// Validation and encoding reject directly constructed noncanonical values.
// Zod trims trimmed text before bounds, but preserves email case and refuses
// surrounding whitespace. These sending constraints also apply when a type is
// received; receive schemas still tolerate unknown object fields.
// +demi:format http-url parses HTTP(S) endpoints using WHATWG rules and stores
// their canonical serialization, including host punycode, default-port removal
// and a slash for an empty path. Validation and encoding require that canonical
// spelling. Zod uses z.url({ protocol: z.regexes.httpProtocol }), as the Rust
// emitter does; its accepted input is canonicalized when decoded by Go.
//
// +demi:table on a package-level slice variable emits its literal struct rows
// into protocol/tables.ts, preserving the variable name and JSON field names.
// Rows must specify every field; values are scalar constants or slice literals,
// with JavaScript-safe integers. Calls, mutable variable references, optional
// fields and maps are refused rather than executed during generation. Each
// scalar or scalar-slice field gets <Table>By<Field>(value), returning the first
// matching row or undefined (slice fields use membership). This supports model
// extension rows and named live-view constant rows without hardcoded values.
// PREVIEW_TYPES additionally emits previewMediaType(path) and
// showsInPlace(mediaType), with ASCII extension case folding and either path
// separator. It must have mediaType, extensions and inPlace fields.
//
// +demi:msgpack enables generated MessagePack codecs throughout the reachable
// shape. They use JSON field names and declaration order, compact integers,
// binary byte slices, sorted string records and timestamp extensions. JSON and
// MessagePack share presence, nullability and validation rules. MessagePack's
// full 64-bit integer domain remains available to non-web roots. On a union,
// +demi:msgpack tuple selects kept-output encoding: one external tag mapped to
// a declaration-order array, or to the field itself for a single-field variant.
// Every tuple field is required (nullable is allowed). Standalone variants use
// the same representation, while JSON retains its internally tagged object.
// The record-stream owner splits records before calling the generated decoder;
// each decoder requires exactly one record and refuses trailing bytes.
//
// +demi:root direction=receive|send output=protocol|web|plugin-<name> selects a
// TypeScript root. Output is optional for roots only decoded by Go. Receive
// propagates through reachable types, producing tolerant Zod objects. Send-only
// objects must be strict. Shared protocol schemas are imported by web and plugin
// outputs. Unknown markers, incompatible rules, ambiguous names and unsupported
// shapes fail with the declaration's position. The generator excludes its own
// output and function bodies while loading declarations, so a clean bootstrap
// does not depend on methods that have not been generated yet.
package main
