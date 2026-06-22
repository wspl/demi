/**
 * Loads the cached large long-session fixture (see build-fixture.ts) and verifies the session is
 * still usable after its many compaction generations: the secrets planted at the very start must
 * have survived being re-summarized each time, and the model must recall them — using the REAL
 * default compaction thresholds, no faking.
 *
 *   bun run scripts/compaction-fixture/verify-compaction.ts
 *
 * Calls the real Claude Code provider — needs `claude` auth and costs a little.
 */
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { gunzipSync } from 'node:zlib'
import type { Block, ModelSelection } from '../../packages/core/src/index'
import { ClaudeCodeProvider } from '../../packages/provider-claude-code/src/provider'
import { AgentSession } from '../../packages/agent/src/index'
import { BashEnvironment, createShellSessionTools } from '../../packages/shell/src/index'
import { LocalHost } from '../../packages/host-local/src/index'

const FIXTURE = join(import.meta.dir, 'large-context-fixture.json.gz')
process.env.DEMI_CLAUDE_WIRE_LOG = '0'

const fx = JSON.parse(gunzipSync(readFileSync(FIXTURE)).toString('utf8')) as {
  harnessName: string
  cwd: string
  model: ModelSelection
  blocks: Block[]
  builtTokens: number
}
const log = (m: string): void => process.stdout.write(`${m}\n`)
const generations = fx.blocks.filter((b) => b.type === 'compaction_boundary').length
log(`loaded long session: total≈${fx.builtTokens} tokens, ${fx.blocks.length} blocks, ${generations} compaction generations`)

const provider = new ClaudeCodeProvider({})
const environment = new BashEnvironment({ host: new LocalHost(fx.cwd), shellIdFactory: () => 'cmp-shell', initialEnv: { PATH: process.env.PATH ?? '' } })
const runtime = {
  harnessName: fx.harnessName,
  initialState: () => ({}),
  systemPrompt: () => 'You are a careful coding assistant. Remember any secrets the user told you verbatim.',
  tools: () => createShellSessionTools(environment),
}
const snapshot = { transcript: { blocks: fx.blocks }, state: {}, phase: 'idle' as const, queue: [], cwd: fx.cwd, model: fx.model, harnessName: fx.harnessName }
// Real default compaction thresholds — no override, no faking.
const session = AgentSession.fromSnapshot({ provider, snapshot, runtime })

log('>>> recall question (the secrets must have survived every compaction generation)')
await session.send([{ type: 'text', text: '只回答暗号值,用「ALPHA=…, BETA=…, GAMMA=…」格式:我最早让你记住的三个暗号分别是什么?' }])

const blocks = session.transcript().blocks
const answer = blocks.filter((b): b is Extract<Block, { type: 'text' }> => b.type === 'text').map((b) => b.text).join(' ')
const errs = blocks.filter((b) => b.type === 'error')
const newGenerations = blocks.filter((b) => b.type === 'compaction_boundary').length - generations
await session.dispose()

const recalled = ['ZEBRA-?7', 'QUARTZ-?9', 'NIMBUS-?3'].filter((re) => new RegExp(re, 'i').test(answer))
log('\n===== LONG-SESSION COMPACTION VERIFY =====')
log(`compaction generations the secrets survived: ${generations}${newGenerations > 0 ? ` (+${newGenerations} more this turn)` : ''}`)
log(`recall: ${recalled.length}/3 secrets`)
log(`error blocks: ${errs.length}`)
const pass = generations >= 2 && recalled.length === 3 && errs.length === 0
log(pass ? '\n✅ PASSED: secrets survived a many-generation compacted session, full recall' : `\n❌ FAILED  answer tail: ${answer.slice(-160)}`)
process.exit(pass ? 0 : 1)
