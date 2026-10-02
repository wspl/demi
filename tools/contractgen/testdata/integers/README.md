# Numbered tool input reference

`rust-schema.json` was emitted by schemars 1.2.2 with serde_json's
`preserve_order` feature, using `SchemaSettings::draft2020_12()` with
`meta_schema = None`, as in `crates/agent-tools/src/input.rs::schema`.
The fixture combines the two relevant properties from Rust's tool inputs:

```rust
#[derive(JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    #[schemars(with = "u64")]
    command_id: String,
    #[serde(default, deserialize_with = "some_numbered")]
    #[schemars(with = "u64")]
    shell_id: Option<String>,
}
```

`command_id` uses the annotation from `CommandInput`; `shell_id` uses the
annotations from `ShellExecInput`. The Rust test
`a_schema_declares_integer_windows_string_handles_and_nothing_else` confirms
the integer schema and optional presence. The capture additionally pins
schemars's `format`, `minimum`, and optional `default: null` annotation.
The test compares complete bytes after sorting object keys, matching the
existing generator's schema ordering. It retains the root title: Rust's tool
caller removes that title, whereas the generic Go schema generator retains it.
No property annotation is removed or changed for the comparison.

`TestIntegerStrings` uses the `u64::from_str` rules called by `NumberVisitor`.
Its complete string table was also checked with a standalone Rust program
calling `s.parse::<u64>()`; it needs no Rust compiler when the Go tests run.
The signed fixture covers the marker's same-integer-type rule and validation
after parsing. All emitted values remain integers.
