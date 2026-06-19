# Real Project Build Marathon Acceptance Record

| | |
|---|---|
| Date | 2026-06-19 |
| Status | Passed |
| Scope | Real TUI + real Claude Code provider + real frontend project |
| Primary model | `claude-haiku-4-5` for cost runs; optionally `claude-opus-4-8` medium thinking for higher-fidelity runs |
| TUI command | `bun run packages/tui/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 2.00 --yield-after-ms 1000 --timeout-ms 180000` |
| Acceptance target | Complete a fuzzy Vue + Pinia todo list task end to end and verify real project commands pass |

## Scenario Design

Use a deliberately imperfect user request, for example: initialize a Vue project and write a Vue + Pinia todo list. Do not include the exact scaffold flags or implementation. The model should choose non-interactive defaults, create files, run install/build/test or equivalent verification, and fix errors.

## Machine-Checkable Evidence

- Project files are created in the workspace.
- A Pinia store exists and is used by the app.
- Todo list supports add, toggle, delete, and filter or persistence if the model chooses it.
- Build or test command exits successfully.
- TUI shows real tool output and returns to idle.

## Pass Criteria

- The generated app builds successfully.
- The requested Vue + Pinia todo list exists in code, not only in prose.
- The model handles scaffold prompts without manual rescue.
- No stuck long process remains after the run.

## Failure Signals

- The model only writes a plan.
- Scaffold gets stuck on interactive prompts.
- The model uses destructive workspace commands.
- Build fails and the model claims success.
- Dev server or watcher remains running after completion.

## Process Record

### Run 1

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-tui-real-project-gbawof`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-tui-real-project-gbawof/tui-real-project-build.log`
- Log size: 101,617 bytes.
- Prompt: `在这个空目录里初始化一个 Vue 项目，并写一个基于 Vue + Pinia 的 todolist。需求故意比较粗略，你自己做合理选择；最后需要自己运行必要的验证，确保项目可以构建或测试通过。`
- Commands observed: initial `npm create vue@latest` interactive scaffold stalled; the model used `shell_input`, then `shell_wait`, then `shell_abort`, recovered with non-interactive `npm create vite@latest . -- --template vue-ts`, installed dependencies, wrote app files, ran build, started and aborted a dev server, ran a store test, and produced final summary output.
- Created files: `src/stores/todoStore.ts`, `src/components/TodoList.vue`, `src/components/TodoItem.vue`, `src/App.vue`, `src/main.ts`, `README.md`, `PROJECT_SUMMARY.md`, `test-store.js`, `dist/`.
- TUI verification: `npm run build` exited 0 inside the TUI run; `node test-store.js` exited 0 with 9 assertions passed.
- Independent verification:
  - `/Users/plutonist/.vite-plus/js_runtime/node/24.16.0/bin/npm run build` exited 0.
  - `./node_modules/.bin/vue-tsc -b --pretty false` exited 0.
  - `./node_modules/.bin/vite build` exited 0.
  - `node test-store.js` exited 0.
- TUI usage: first response `in=10190 out=23668 cache_read=595002 cache_write=68498`, then auto compact, then resume response `in=10 out=628 cache_read=0 cache_write=15036`.
- Phase evidence: `status: running`, `status: compacting`, resumed `status: running`, final `status: idle`.
- Verdict: Passed.

## Failure Analysis

No acceptance failure remained in the passing run.

Important observations:

- The model first chose an interactive scaffold path and got stuck on package-name input; it recovered by aborting the foreground process and switching to non-interactive Vite scaffold.
- Extra test dependency installation timed out; the model aborted and fell back to a local store verification script. This is acceptable for this acceptance because the primary build and functional verification passed.
- Independent rerun with Homebrew `npm` exited 137 in this workspace, while the TUI's Node/npm runtime and direct `vue-tsc`/`vite build` commands passed. The verdict uses the TUI runtime and direct build tools as authoritative for this run.

## Follow-Up Deterministic Tests

Failures should become `coding-agent` scenario tests, shell foreground-control tests, or TUI process tests depending on the broken layer.
