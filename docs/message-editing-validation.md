# Message editing checkpoint validation

Validated on 2026-09-11. The current implementation contract is
[Message editing and resend](message-editing.md). All provider requests in this
validation used scripted providers or fake transports. No real model was called.

## Automated results

The agent, backend, coding harness, core, provider and Claude Code fake-transport
suites passed **590 tests**, with **12 environment-gated skips** and no failures
(3,816 assertions). The web and shared UI suites passed **197 tests**, with no
failures (648 assertions).

Commands run from the repository root:

```sh
CMAKE=/tmp/demi-txiki-build-tools/cmake/data/bin/cmake DEMI_FIRECRACKER_E2E=0 bun test --conditions development ./packages/agent/src/__tests__ ./packages/backend/src/__tests__ ./packages/coding-agent/src/__tests__ ./packages/core/src/__tests__ ./packages/provider/src/__tests__ ./packages/provider-claude-code/src/__tests__/provider.test.ts
bun test --conditions development ./packages/web-ui/src ./packages/web/src
bun run typecheck
bun --filter @demicodes/core build
bun --filter @demicodes/agent build
bun run typecheck:web
bun run web:build
```

`bunx vite build` also passed from `packages/web-gallery`. Both Vite builds
reported their existing large-chunk warnings. The `CMAKE` path above selects the
local native-runner test toolchain; it is an execution record, not a portable
installation path.

## Evidence by boundary

| Boundary | Checked result |
| --- | --- |
| Transcript and replay | First, middle and final edits; fixed-seed histories; exact retained prefix; detached multipart content; freshly resolved references; removed compaction markers; multiple summaries; preflight compaction. Session blocks, applied client patches, loaded checkpoints and independently expected provider inputs agree. |
| Admission and resources | Preparation and save barriers, both commit/abort outcomes, disposal during save, duplicate requests, stale snapshots, pending actions/wakeups, child records and both child-restoration race orders. Failed preparation publishes no candidate or turn-recovery error. |
| Provider context | The real Claude Code provider over fake transports starts a fresh transport for a final non-first edit and for an attachment-only byte change. The consumed transport receives no continuation and is released. |
| Durable storage | Actual SQL failure after block writes/deletion rolls back the transaction. Blob failure preserves the checkpoint. An earlier scheduled checkpoint cannot overwrite the edit. |
| Process termination | A separate scripted session process using backend SQLite/blob stores receives SIGKILL inside the transaction, after commit before publication, or after output persistence. Reopen observes the appropriate complete checkpoint and receipt. Submission reconciliation does not start inference; explicit recovery is separate. |
| Authenticated backend | A real backend scenario edits a three-turn conversation, takes over its connection, restarts the backend and reconciles the original operation. A sentinel file and command-store todo created in the removed suffix remain intact. |
| Protocol | Missing replacement patches trigger snapshot resynchronization. Lost acceptance frames reconcile across reconnect. Unrelated matching text does not confirm an edit. Malformed requests, archived writes, invalid targets and stale requests are rejected. Existing authenticated-route regressions cover conversation ownership. |
| Shared UI and product data | Pending saves block duplication; uncertain requests retain their identity across reload; explicit rejection restores editing; late replies cannot clear another draft. Acceptance reconciliation preserves an existing generation error and its recovery action. Media hydration preserves two same-name files with different bytes. The keyboard predicate rejects Enter during IME composition. |

The child-restoration fixture now routes its script by request content and waits
for child startup through a deferred barrier. A slow native-runner startup can
no longer let parent continuation consume the child's scripted response.

## Browser acceptance

A disposable authenticated backend supplied only scripted replies. Product
acceptance checked the visible page and captured provider requests:

- Cancel preserved the original history and an unrelated composer draft.
- Closing and reloading retained both edited text parts and two distinct PDF
  attachments named `same.pdf`.
- Chinese and multiline text survived editing. Ctrl+Enter entered the disabled
  saving state, then replaced B and its suffix while preserving A and answer A.
- The captured fresh-runtime request contained exactly the retained A context,
  both replacement text parts and both distinct PDF byte payloads. It contained
  no removed B/C answers. Reload recovered that same accepted history and the
  unrelated composer draft.
- Gallery used the same editor for held save, completion after closing the
  dialog, explicit conflict, lost confirmation and Retry. Button activation
  submitted correctly; confirmation retry did not apply a second replacement.

Native IME candidate selection was not automated in the browser; the composing
keyboard-event guard was verified in the shared unit suite.

## Defect sensitivity

Each temporary mutation caused its corresponding focused assertion to fail:

1. Retaining the old target block in the candidate prefix.
2. Reusing the consumed provider runtime.
3. Publishing the replacement before the save completed.
4. Skipping transcript epoch/revision validation.

All mutations were removed before the final suites above. Their clean rerun
passed. This check establishes sensitivity to these four defects; it is not
an exhaustive proof of all possible failures.
