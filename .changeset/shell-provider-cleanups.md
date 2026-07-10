---
"@demicodes/core": patch
"@demicodes/agent": patch
"@demicodes/shell": patch
"@demicodes/repl": patch
"@demicodes/provider-codex": patch
"@demicodes/provider-claude-code": patch
"@demicodes/provider-grok-build": patch
---

Residue cleanup: `supportedAssetTypesFor(model)` in core replaces two inline
ternaries; the codex/grok text redactors get unambiguous vendor names
(`redactCodexSecretText`, private grok equivalent) instead of shadowing the
provider kit's differently-typed `redactSecretText`; claude-code quota parses
the unified-utilization header via the shared `numberHeader`; leftover
`editor` naming from the demi rename is gone from comments, docs, and test
fixtures; and the real-spawn exclusions (`bash`/`sh`/`sleep`) are documented
as a routing decision rather than a wrapper workaround.
