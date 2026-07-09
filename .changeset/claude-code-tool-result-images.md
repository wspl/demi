---
"@demicodes/provider-claude-code": patch
---

Forward images returned by tools to the Claude Code CLI instead of dropping them.

A `tool_result` carrying an `image` block was being flattened to a `[image:…]`
text placeholder on both paths: the live SDK-MCP tool-call response
double-encoded the base64 `data`, and the replayed-history serialization
replaced the image with placeholder text. The Claude Code CLI does accept images
inside `tool_result` content, so both now pass the image through unchanged — the
model can actually see images a tool returns.
