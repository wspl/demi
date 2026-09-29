# Runner protocol corpus

A scratch Rust program in `/tmp/astra-l5/rust` linked
`/home/user/demi/crates/runner-protocol` by path. It read the Rust crate's
fixture messages into `Inbound` and `Outbound`, validated them, and wrote
`wire::encode` bytes. The corpus has 119 frames, covering every message variant
and every FS/Git reply operation. Additional edge frames contain all 256 byte
values, maximum `u64`, integral and fractional floats, negative zero, and timestamps of -1 and maximum `i64` milliseconds.

The manifest includes its canonical hash. Rust `Manifest::parse` verifies it;
Go rebuilds it from its declarations and package descriptors and checks the same
hash. The release and managed boot records were encoded by Rust; boot has both
JSON and MessagePack fixtures.

`kept/output.msgpack` uses the Rust crate's `wire::encode_record`, which
calls `rmp_serde::to_vec`: an externally tagged `output` with a positional
stream/binary tuple, or `left_out` with an unsigned count. The records contain
stdout bytes, a maximum-u64 gap, and stderr bytes. Go uses only this codec;
there is no generated named form or Rust serde mirror.

`negative-zero.json` is parsed by Rust's `serde_json` and encoded with
`rmp_serde::to_vec_named` into `negative-zero.msgpack`. It includes lexical
`-0` as well as `-0.0`, integer zero, and floating-point zero.

`TestMessageCorpus` decodes and re-encodes every frame byte-for-byte.
`protocol_test.go` checks the other records. No test builds or invokes Rust.
For the reverse check, these tests export Go-encoded records when
`DEMI_RUNNER_CORPUS_OUTPUT` names a directory. The scratch Rust program decoded
all those exported frames with `wire::decode`, verified the manifest, and
re-encoded them to identical bytes. Its actual `wire::decode_records` and `wire::encode_record` likewise
checked kept records; opaque negative zero was checked through `serde_json::Value`.
Both directions passed.

Rust builds ran through `heavy.sh`, with the isolated
`CARGO_TARGET_DIR=/tmp/astra-l5/target`. The Go checks used the repository's
normal `go.mod`, without a temporary module or workspace.
