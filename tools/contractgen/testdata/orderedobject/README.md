`rust.jsonl` is emitted by `/Users/zan/Projects/demi-worktrees/gomig-ref/oracles/f-contractgen8/src/main.rs`, using serde_json with preserve_order and schemars 1.2.2. It contains the schema followed by non-object decoder refusals. Go keeps the field path and message; source line and column are not part of its contract errors.

`encodings.jsonl` comes from the same oracle with `--encodings`. It records the parsed object and containing contract serialized by serde_json, including whitespace, exponent notation, negative zero, integer precision and escaped characters.

`numbers.json` comes from `--numbers` and pins typed float32 serialization through a scalar, a list and a containing contract, independently of parsed JSON number normalization.
