# Compaction fixture

Compaction and a switch to a smaller context window need a genuinely large
conversation, and building one costs hundreds of thousands of real tokens. So
one such session was built once, with three secrets planted at its start, and
is replayed here against the real model to check that the secrets survive.

`large-context-fixture.json.gz` is that session: 237 blocks that compacted
four times over about 1.08 million tokens, with its harness name and working
directory, in the agent's block format. It was converted once from the
TypeScript fixture it replaces (`packages/agent/fixtures/compaction/` at commit
`1b7fdbab`): each block's model selection gained its service tier and output
limit, a `user` block its turn id (its own id), a `resume` block the turn of
the `user` block before it, a completed `text` block its `forkable` mark, and a
`tool_call` block its view, while the TypeScript-only `metadata` and
`streamingOutput` went. The harness loads it with the agent's own decoding, so
a block that does not decode stops it.

`main.rs` opens the fixture as a conversation of an agent server over an
in-memory tree store and talks to the real DeepSeek V4 Flash through the
OpenAI-compatible provider. It is a program to run by hand: no build of the
one selection compiles it, and no test runs it.

```sh
export DEEPSEEK_API_KEY=...
cargo run -p demi-agent --features compaction-fixture --example compaction-fixture -- recall
cargo run -p demi-agent --features compaction-fixture --example compaction-fixture -- switch
```

| Mode | What it checks |
| --- | --- |
| `recall` | The secrets recall 3/3 on a 200,000-token window; then three times: filler, a forced `compact`, which must add a compaction generation, and recall 3/3 again. |
| `switch` | Small window to large: no compaction, recall 3/3. Filler until the small window's threshold is reached. Large to small: the model before the switch compacts first, recall 3/3. Back to large: recall 3/3. |

Either mode prints a table and exits non-zero when a check fails or an
`error` block appears.

| Variable | Default | Meaning |
| --- | --- | --- |
| `DEEPSEEK_API_KEY` | — | Required |
| `DEEPSEEK_BASE_URL` | `https://api.deepseek.com/v1` | The API's base |
| `DEEPSEEK_FLASH_MODEL` | `deepseek-v4-flash` | The model id |
| `COMPACTION_FIXTURE_SMALL_WINDOW` | `8000` | `switch`'s small window |
| `COMPACTION_FIXTURE_LARGE_WINDOW` | `400000` | `switch`'s large window |

The model runs without tools: the standard tools arrive with the shell
environments they run in, and recall needs none. The fixture's own tool calls
are history the model reads.
