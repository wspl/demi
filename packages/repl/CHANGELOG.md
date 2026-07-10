# @demicodes/repl

## 0.0.1

### Patch Changes

- 2af7114: Residue cleanup: `supportedAssetTypesFor(model)` in core replaces two inline
  ternaries; the codex/grok text redactors get unambiguous vendor names
  (`redactCodexSecretText`, private grok equivalent) instead of shadowing the
  provider kit's differently-typed `redactSecretText`; claude-code quota parses
  the unified-utilization header via the shared `numberHeader`; leftover
  `editor` naming from the demi rename is gone from comments, docs, and test
  fixtures; and the real-spawn exclusions (`bash`/`sh`/`sleep`) are documented
  as a routing decision rather than a wrapper workaround.
- Updated dependencies [8b7b981]
- Updated dependencies [9179edc]
- Updated dependencies [32f0e41]
- Updated dependencies [3360e35]
- Updated dependencies [bf2ffa2]
- Updated dependencies [966c530]
- Updated dependencies [0bcb313]
- Updated dependencies [d203fc1]
- Updated dependencies [579e231]
- Updated dependencies [084831e]
- Updated dependencies [10dbc6b]
- Updated dependencies [09bcc0d]
- Updated dependencies [18a72d1]
- Updated dependencies
- Updated dependencies [80d5c6d]
- Updated dependencies [2af7114]
  - @demicodes/utils@0.2.0
  - @demicodes/core@0.2.0
  - @demicodes/shell@0.2.0
  - @demicodes/agent@0.2.0
  - @demicodes/coding-agent@0.2.0
  - @demicodes/host-local@0.2.0
  - @demicodes/provider-claude-code@0.2.0
  - @demicodes/provider@0.2.0
  - @demicodes/provider-codex@0.2.0
  - @demicodes/provider-grok-build@0.2.0
  - @demicodes/provider-openai-api@0.2.0
  - @demicodes/provider-anthropic-api@0.2.0
