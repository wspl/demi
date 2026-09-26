# Brief F4c: agent protocol contracts

Same agent and slot as F4b (s2), continued on branch go/wp/F4b-contracts after
F4b's verification fixes, or on go/wp/F4c-agentproto from go/main if F4b has
merged by then (the orchestrator says which). Rules: _rules.md.

Design (merged on go/main, commit 6eb8299c): docs/demi-next/native-runtime.md
§ Contract generation and validation, the paragraph on checks expressed as
code and on core's Go types.

Deliver:
1. go-zod support for z.tuple, z.iso.datetime() (the exact Zod acceptance),
   z.instanceof(Uint8Array) (bytes), z.instanceof(Date) (time), z.bigint()
   (math/big) in portable JSON.
2. Named code checks: `.meta({ check: '<name>' })` on every refine/z.custom in
   packages/agent/src/protocol (agent-message.ts: the two agentMessage checks;
   schemas.ts: the edit-request content check, the agent_message receipt id
   check, fileExtensionSchema). Generation fails for an unnamed code check.
   Hand-written Go in internal/contract/agentproto/checks.go; completionMessageId
   is ported there as the one Go owner of that id rule.
3. Generate internal/contract/agentproto from packages/agent/src/protocol,
   reading wireBlockSchema where blockSchema only casts its type. No
   internal/contract/core package: core's types are these.
4. Corpus and rejection cases as in F4b, including cases that exercise each
   named check both ways.
Owned paths: as F4b plus the `.meta` additions in packages/agent/src/protocol
(metadata only; no behavior change; the TS tests of packages/agent must still
pass: `bun test --conditions development packages/agent`).
