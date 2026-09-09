import {
  DEMO_BUN_TEST,
  DEMO_GIT_DIFF,
  DEMO_RG,
} from '@demicodes/web-ui/agent/terminal-demo'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'

function ago(ms: number): string {
  return new Date(Date.now() - ms).toISOString()
}

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
  ]
}
