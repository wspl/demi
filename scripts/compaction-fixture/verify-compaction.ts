/**
 * Loads the cached large-context fixture (see build-fixture.ts) and triggers ONE real compaction
 * with a realistic threshold, then asks the model to recall the planted secrets. Verifies the
 * compaction produced a real, non-empty summary and that recall still works afterward.
 *
 *   bun run scripts/compaction-fixture/verify-compaction.ts
 *
 * Calls the real Claude Code provider — needs `claude` auth and costs a little.
 */
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import type { Block, ModelSelection } from '../../packages/core/src/index'
import { ClaudeCodeProvider } from '../../packages/provider-claude-code/src/provider'
import { AgentSession } from '../../packages/agent/src/index'
import { BashEnvironment, createShellSessionTools } from '../../packages/shell/src/index'
import { LocalHost } from '../../packages/host-local/src/index'

const FIXTURE = join(import.meta.dir, 'large-context-fixture.json')
process.env.DEMI_CLAUDE_WIRE_LOG = '0'

const fx = JSON.parse(readFileSync(FIXTURE, 'utf8')) as {
  harnessName: string
  cwd: string
  model: ModelSelection
  blocks: Block[]
  builtTokens: number
}
const log = (m: string): void => process.stdout.write(`${m}\n`)
log(`loaded fixture: tokens≈${fx.builtTokens} blocks=${fx.blocks.length}`)

const provider = new ClaudeCodeProvider({})
const environment = new BashEnvironment({ host: new LocalHost(fx.cwd), shellIdFactory: () => 'cmp-shell', initialEnv: { PATH: process.env.PATH ?? '' } })
const runtime = {
  harnessName: fx.harnessName,
  initialState: () => ({}),
  systemPrompt: () => 'You are a careful coding assistant. Remember any secrets the user told you verbatim.',
  tools: () => createShellSessionTools(environment),
}

const snapshot = { transcript: { blocks: fx.blocks }, state: {}, phase: 'idle' as const, queue: [], cwd: fx.cwd, model: fx.model, harnessName: fx.harnessName }
// Trigger at ~60% of the fixture, keep ~8% recent: one clean compaction, no storm.
const ratio = (fx.builtTokens * 0.6) / fx.model.model.contextWindow
const keepRecentTokens = Math.max(500, Math.floor(fx.builtTokens * 0.08))
const session = AgentSession.fromSnapshot({ provider, snapshot, runtime }, { compaction: { preflightThresholdRatio: ratio, keepRecentTokens } })

log('>>> recall question (forces preflight compaction on the loaded context, then answers)')
await session.send([{ type: 'text', text: '只回答暗号值,用「ALPHA=…, BETA=…, GAMMA=…」格式:三个暗号分别是什么?' }])

const blocks = session.transcript().blocks
const boundaries = blocks.filter((b): b is Extract<Block, { type: 'compaction_boundary' }> => b.type === 'compaction_boundary')
const answer = blocks.filter((b): b is Extract<Block, { type: 'text' }> => b.type === 'text').map((b) => b.text).join(' ')
const errs = blocks.filter((b) => b.type === 'error')
await session.dispose()

log('\n===== REAL COMPACTION (from cached fixture) =====')
log(`compaction boundaries created: ${boundaries.length}   (1-2 = clean; many = storm)`)
boundaries.forEach((b, i) => {
  const s = b.summary || ''
  const secrets = [/ZEBRA-7/.test(s) && 'ALPHA', /QUARTZ-9/.test(s) && 'BETA', /NIMBUS-3/.test(s) && 'GAMMA'].filter(Boolean).join(' ')
  log(`  boundary #${i + 1}: summary length=${s.length}  secrets=[${secrets}]`)
})
const recalled = ['ZEBRA-?7', 'QUARTZ-?9', 'NIMBUS-?3'].filter((re) => new RegExp(re, 'i').test(answer))
const pass = boundaries.length >= 1 && boundaries.length <= 4 && boundaries.every((b) => (b.summary || '').length > 50) && recalled.length === 3 && errs.length === 0
log(`recall after compaction: ${recalled.length}/3 secrets`)
log(`error blocks: ${errs.length}`)
log(pass ? '\n✅ PASSED: clean compaction, real summary, full recall' : `\n❌ FAILED  answer tail: ${answer.slice(-160)}`)
process.exit(pass ? 0 : 1)
