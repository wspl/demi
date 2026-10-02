# serde_json reference bytes

`escaping.jsonl` was captured from serde_json 1.0.151, the Cargo.lock version,
with `cargo run --quiet --offline --manifest-path oracle/Cargo.toml` (stdout).
The small oracle is retained for reproducibility; Go tests only read the
captured fixture and never build Rust. It covers HTML characters, JavaScript
line separators, quotes/backslashes, every ASCII control, non-ASCII text,
and literal backslash escapes.

The oracle also reports on stderr that 126 and 127 nested arrays decode,
while 128 and 129 do not. In `src/de.rs`, `remaining_depth` starts at 128;
`check_recursion!` decrements before entering an object or array and refuses
zero. Scalars consume no recursion budget. This differs from the brief's
example that 128 containers decode; the runtime follows the Rust source.
