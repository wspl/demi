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
// +demi:msgpack enables generated MessagePack codecs throughout the reachable
// shape. They use JSON field names and declaration order, compact integers,
// binary byte slices, sorted string records and timestamp extensions. JSON and
// MessagePack share presence, nullability and validation rules. MessagePack's
// full 64-bit integer domain remains available to non-web roots.
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
