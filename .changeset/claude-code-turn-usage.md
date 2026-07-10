---
'@demicodes/provider-claude-code': patch
'@demicodes/provider': patch
'@demicodes/agent': patch
---

Report single-request usage instead of turn-cumulative totals from Claude Code.

The CLI's `result.usage` sums every API call inside a turn, which inflated the
agent's context estimation 2–3× and triggered spurious compaction on long
tool-heavy sessions. The provider now maps the last `usage.iterations[]` entry
(the final request's real usage) as the response usage, the provider `response`
event documents the single-request contract, and `estimateContextTokens`
discards anchors larger than the context window as physically impossible.
