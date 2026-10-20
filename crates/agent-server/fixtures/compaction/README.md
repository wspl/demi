# Compaction fixture

Compaction needs a genuinely large conversation, and building one costs
hundreds of thousands of real tokens. So one such session was built once and
is replayed here against the real DeepSeek: the real-model check of
compaction, the third level of its verification.

`large-context-fixture.json.gz` is that session: 237 blocks that compacted
four times over about 1.08 million tokens, with its working directory, in the
agent's block format. It was converted once from the TypeScript fixture it
replaces (`packages/agent/fixtures/compaction/` at commit `1b7fdbab`): each
block's model selection gained its service tier and output limit, a `user`
block its turn id (its own id), a `resume` block the turn of the `user` block
before it, a completed `text` block its `forkable` mark, and a `tool_call`
block its view, while the TypeScript-only `metadata` and `streamingOutput`
went; later its harness name went with the harness, and each model's input
limit with the model's. The fixture loads it with the agent's own decoding,
so a block that does not decode stops it; when the block format changes, the
file changes with it.

## What a run does

The harness loads a copy of the session and changes two things in it; the
file on disk stays as it is:

- It removes the four recorded compactions, so the run starts from the whole
  history, about 212,000 tokens as DeepSeek counts them, and its own passes
  summarize the start.
- It plants, in the first message beside the three secrets
  (`ZEBRA-7`, `QUARTZ-9`, `NIMBUS-3`), a constraint ("never edit files under
  `migrations/`; ask first") and an open task (`TASK-17`, a CHANGELOG entry).

`main.rs` opens the copy as a conversation of an agent server over an
in-memory tree store, with the product's compaction settings, and talks to
DeepSeek through the OpenAI-compatible provider. It observes compaction only
through the transcript (each new `compaction_boundary`), the session's
context usage and the usage the provider reports, so it holds no copy of the
thresholds or the cut point. It is a program to run by hand, and no test
runs it. It is the agent crate's example, so every build and test of the one
selection compiles it (`cargo test` builds every example), which keeps it in
step with the agent's API, and puts it in `target/debug/examples`:

```sh
export DEEPSEEK_API_KEY=...
target/debug/examples/compaction-fixture recall [--passes N]
target/debug/examples/compaction-fixture switch [--passes N]
```

The development endpoint in the repository's `.env` serves DeepSeek too:

```sh
set -a; . ./.env; set +a
DEEPSEEK_API_KEY="$DEMI_DEV_PROVIDER_API_KEY" \
DEEPSEEK_BASE_URL="$DEMI_DEV_PROVIDER_BASE_URL" \
DEEPSEEK_FLASH_MODEL="$DEMI_DEV_PROVIDER_MODEL" \
  target/debug/examples/compaction-fixture recall
```

| Mode | What it does |
| --- | --- |
| `recall` | Checks on the large window, where nothing may compact. Pass 1 is the switch to the small window, which compacts with the large window's model. Each later pass sends filler turns until the session admits a `compact`, then compacts; a pass the session runs by itself while the filler grows counts too. |
| `switch` | Each pass switches to the large window, where nothing may compact, sends filler turns until the history is larger than the small window, and switches back to the small window, which must compact with the large window's model. The first pass checks on the large window before it fills. |

A switch takes effect, and compacts, at the conversation's next request, so
each switch lands with a turn of its own in the conversation, "reply only
`ok`".

`--passes` sets the number of passes, 3 by default. The filler turns quote
the fixture's own tool output, about 12,000 tokens a turn, and ask for `ok`.

## The checks

After each pass, and on the large window before the first, the harness
asks three short questions and checks the answers. Each question goes to a
throwaway copy of the conversation as it stands, stored as a fork is, with
the whole transcript, and closed after its answer. So the conversation's
history never holds a check: a later pass cannot carry the secrets, the
constraint or the task from a check's answer, only from what the earlier
summaries kept. The filler and landing turns stay in the conversation.

| Message | Passes when |
| --- | --- |
| "What next? Name the unfinished task's number, if any." | The answer names `TASK-17`. After a pass, it is the first question after the summary, and its answer must not restate the summary: it does not open with "Summary", "Here is a summary", "总结" or the like, does not quote `<summary>`, and is not mostly lines of the summary. The conversation's own turn that ran the pass, a landing or filler turn, is held to the same rule. |
| "Add an index in `migrations/0042_users.sql`. Answer `EDIT` or `ASK` and the rule." | The answer opens with `ASK` and names `migrations`. |
| "What were the three secrets?" | It names all three. |

The answer to what next, and the turn that ran the pass, are printed for a
person to judge as well, and a failed check prints its turn: the question,
the reasoning and the answer.

For each summary request, which the harness finds as the recorded request
whose answer became the new boundary's summary, it prints the usage DeepSeek
reported: input, cache read and cache write. When the request without its
instruction repeats an earlier request of the run, whole or as a prefix, the
vendor's cache holds it, and a cache read of zero fails the step.

Either mode prints one table, with the run's total token usage, and exits
non-zero when a check fails or an `error` block appears.

| Variable | Default | Meaning |
| --- | --- | --- |
| `DEEPSEEK_API_KEY` | — | Required |
| `DEEPSEEK_BASE_URL` | `https://api.deepseek.com/v1` | The API's base |
| `DEEPSEEK_FLASH_MODEL` | `deepseek-v4-flash` | The model id |
| `COMPACTION_FIXTURE_SMALL_WINDOW` | `128000` | The small window |
| `COMPACTION_FIXTURE_LARGE_WINDOW` | `1000000` | The large window; it must hold the whole history |

The model runs without tools: the standard tools arrive with the shell
environments they run in, and the checks need none. The fixture's own tool
calls are history the model reads.
