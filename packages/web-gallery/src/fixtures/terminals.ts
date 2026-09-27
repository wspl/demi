import { DEMO_BUN_TEST, DEMO_GIT_DIFF, DEMO_RG } from './terminal-output'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import { runningShellTool } from './blocks'
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
      name: 'bun test',
      phase: 'running',
      startedAt: ago(60_000),
      output: DEMO_BUN_TEST,
    },
    {
      id: 'term-rg',
      name: 'rg sid',
      phase: 'running',
      startedAt: ago(45_000),
      output: DEMO_RG,
    },
    {
      id: 'term-diff',
      name: 'git diff',
      phase: 'running',
      startedAt: ago(30_000),
      output: DEMO_GIT_DIFF,
    },
    {
      id: LIVE_TERMINAL_ID,
      name: 'bun test --watch packages/web/src/auth.test.ts',
      phase: 'running',
      startedAt: ago(20_000),
      output: '',
      chars: 0,
      toolUseId: runningShellTool.toolUseId,
    },
  ]
}
