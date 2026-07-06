import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { expect, test } from 'bun:test'
import { createClaudeCodeProvider } from '@demicodes/provider-claude-code'
import { loadEvalCase } from '../case-schema'
import { runEvalCase } from '../runner'

// Gated real-provider smoke: run with DEMI_CLAUDE_CODE_EVAL=1 and a working
// claude CLI. Never part of default CI (real model behavior is nondeterministic).
const gated = process.env.DEMI_CLAUDE_CODE_EVAL !== '1'

test.skipIf(gated)('claude-code fixes the failing test case end to end', async () => {
  const packageRoot = join(dirname(fileURLToPath(import.meta.url)), '..', '..')
  const evalCase = await loadEvalCase(join(packageRoot, 'cases', 'coding', 'fix-failing-test.json'))
  const result = await runEvalCase({ evalCase, providers: [createClaudeCodeProvider()] })

  expect(['pass', 'partial']).toContain(result.finalStatus)
}, 600_000)
