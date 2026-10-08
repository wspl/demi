import { DEMO_BUN_TEST, DEMO_GIT_DIFF, DEMO_RG } from './terminal-output'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import { RUNNING_SHELL_DESCRIPTION, RUNNING_SHELL_SCRIPT, runningShellTool } from './blocks'
import { ago } from './time'

/**
 * The live command: the running call of the demo transcript started it, so
 * its output shows under that call, and it prints as it runs
 * (`useLiveGalleryCommand`).
 */
export const LIVE_TERMINAL_ID = 'term-watch'

export function galleryTerminals(): TerminalRecord[] {
  return [
    {
      id: 'term-test',
      title: 'Run the auth tests',
      script: 'bun test src/auth.test.ts',
      phase: 'running',
      startedAt: ago(60_000),
      output: DEMO_BUN_TEST,
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
