import { DEMO_BUN_TEST, DEMO_GIT_DIFF, DEMO_RG } from './terminal-output'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import { BASH_SCRIPT, LEFT_RUNNING_CALL, RUNNING_RUN_CALL, RUNNING_SHELL_DESCRIPTION, RUNNING_SHELL_SCRIPT, returnedShellTool, runningShellTool } from './blocks'
import { ago } from './time'

/**
 * The live command: the running call of the demo transcript started it, so
 * its output shows under that call, and it prints as it runs
 * (`useLiveGalleryCommand`).
 */
export const LIVE_TERMINAL_ID = 'term-watch'

/** The command the running run's newest call started, a minute and a half ago (`runningRunBlocks`). */
export function runningRunCommand(): TerminalRecord {
  return {
    id: 'term-merge',
    title: 'Review and merge the reset wording',
    script: 'git merge --no-ff fix/reset-wording && bun run test',
    phase: 'running',
    startedAt: ago(92_000),
    output: 'Merge made by the \'ort\' strategy.\nbun test v1.3.14\n',
    toolUseId: `${RUNNING_RUN_CALL}-use`,
  }
}

/** The command the ended turn left running, started 2m16s ago (`leftRunningBlocks`). */
export function leftRunningCommand(): TerminalRecord {
  return {
    id: 'term-e2e',
    title: 'Run the end-to-end suite',
    script: 'bun run e2e',
    phase: 'running',
    startedAt: ago(136_000),
    output: 'Running 140 specs\n',
    toolUseId: `${LEFT_RUNNING_CALL}-use`,
  }
}

export function galleryTerminals(): TerminalRecord[] {
  return [
    {
      id: 'term-test',
      title: 'Run the auth tests',
      script: 'bun test src/auth.test.ts',
      phase: 'running',
      startedAt: ago(60_000),
      output: DEMO_BUN_TEST,
      toolUseId: returnedShellTool.toolUseId,
    },
    {
      id: 'term-rg',
      title: 'Find where the old cookie name is still used',
      // Longer than the terminal is wide: it wraps as a terminal wraps what was typed.
      script: "rg --line-number --color=always --glob '!**/*.snap' --glob '!**/fixtures/**' --max-columns 200 sid packages/web/src packages/web-ui/src packages/web-gallery/src",
      phase: 'running',
      startedAt: ago(45_000),
      output: DEMO_RG,
    },
    {
      id: 'term-diff',
      title: 'Show the auth test changes',
      script: 'git diff --stat\ngit diff packages/web/src/auth.test.ts',
      phase: 'running',
      startedAt: ago(30_000),
      output: DEMO_GIT_DIFF,
    },
    {
      id: 'term-bash',
      title: 'Count the old cookie name and list the changed files',
      script: BASH_SCRIPT,
      phase: 'running',
      startedAt: ago(20_000),
      output: '4\nchanged: packages/web/src/auth.test.ts\nchanged: packages/web/src/cookie.ts\n',
    },
    {
      id: LIVE_TERMINAL_ID,
      title: RUNNING_SHELL_DESCRIPTION,
      script: RUNNING_SHELL_SCRIPT,
      phase: 'running',
      startedAt: ago(20_000),
      output: '',
      chars: 0,
      toolUseId: runningShellTool.toolUseId,
    },
  ]
}
