# serde_json reference bytes

`escaping.jsonl` was captured from serde_json 1.0.151, the Cargo.lock version,
by a small Rust oracle run once during the migration (stdout); the oracle is
kept outside the repository, in the migration's reference directory
(`gomig-ref/oracles/contract/oracle`), since the repository holds no Rust. Go
tests only read the captured fixture. It covers HTML characters, JavaScript
line separators, quotes/backslashes, every ASCII control, non-ASCII text,
and literal backslash escapes.

The oracle also reports on stderr that 126 and 127 nested arrays decode,
while 128 and 129 do not. In `src/de.rs`, `remaining_depth` starts at 128;
`check_recursion!` decrements before entering an object or array and refuses
zero. Scalars consume no recursion budget. This differs from the brief's
example that 128 containers decode; the runtime follows the Rust source.

`numbers.jsonl` comes from the same command with `-- --numbers`. It pins
serde_json's integer limits, overflow-to-float behavior, negative zero, float
notation thresholds, subnormal values and the largest finite double. Go tests
convert these JSON values to MessagePack and back without running Rust.
