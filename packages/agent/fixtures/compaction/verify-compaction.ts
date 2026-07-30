/**
 * Loads the cached large-context fixture and checks that secrets planted before the
 * first compaction still recall under DeepSeek V4 Flash at real default thresholds.
 *
 *   bun run fixtures:compaction:verify
 *
 * Requires `DEEPSEEK_API_KEY` in the repo-root `.env`.
 */
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { gunzipSync } from 'node:zlib'
import type { Block } from '@demicodes/core'
import { AgentSession, createStandardAgentTools } from '../../src/index'
import { BashEnvironment } from '@demicodes/shell'
import { LocalHost } from '@demicodes/host-local'
import { createDeepSeekFlash } from './deepseek'

const FIXTURE = join(import.meta.dir, 'large-context-fixture.json.gz')
/** Registered window for the recall turn; shrink to force preflight if the fixture is huge. */
const VERIFY_CONTEXT_WINDOW = Number(process.env.COMPACTION_FIXTURE_CONTEXT_WINDOW ?? 200_000)

const fx = JSON.parse(gunzipSync(readFileSync(FIXTURE)).toString('utf8')) as {
  harnessName: string
  cwd: string
  blocks: Block[]
  builtTokens: number
}
const log = (m: string): void => process.stdout.write(`${m}\n`)
const generations = fx.blocks.filter((b) => b.type === 'compaction_boundary').length
log(`loaded fixture: total≈${fx.builtTokens} tokens, ${fx.blocks.length} blocks, ${generations} compaction generations`)

const { runtime: provider, model } = await createDeepSeekFlash(VERIFY_CONTEXT_WINDOW)
const environment = new BashEnvironment({
  host: new LocalHost(fx.cwd),
  shellIdFactory: () => 'cmp-shell',
  initialEnv: { PATH: process.env.PATH ?? '' },
})
let sessionRef: AgentSession<Record<string, never>> | null = null
const runtime = {
  harnessName: fx.harnessName,
  initialState: () => ({}),
  systemPrompt: () => 'You are a careful coding assistant. Remember any secrets the user told you verbatim.',
  tools: () =>
    createStandardAgentTools({
      environment,
      scheduleYield: (_ctx, durationMs) => {
        if (!sessionRef) throw new Error('fixture session is not ready for yield scheduling')
        return sessionRef.scheduleYieldWakeup(durationMs)
      },
    }),
}
const checkpoint = {
  transcript: { blocks: fx.blocks },
  state: {},
  phase: 'idle' as const,
  queue: [],
  cwd: fx.cwd,
  model,
  harnessName: fx.harnessName,
}
const session = AgentSession.fromCheckpoint({ provider, checkpoint, runtime })
sessionRef = session

log(`>>> recall on deepseek-v4-flash (contextWindow=${VERIFY_CONTEXT_WINDOW})`)
await session.send([
  {
    type: 'text',
    text: '只回答暗号值,用「ALPHA=…, BETA=…, GAMMA=…」格式:我最早让你记住的三个暗号分别是什么?',
  },
])

const blocks = session.transcript().blocks
const answer = blocks
  .filter((b): b is Extract<Block, { type: 'text' }> => b.type === 'text')
  .map((b) => b.text)
  .join(' ')
const errs = blocks.filter((b) => b.type === 'error')
const newGenerations = blocks.filter((b) => b.type === 'compaction_boundary').length - generations
await session.dispose()

const recalled = ['ZEBRA-?7', 'QUARTZ-?9', 'NIMBUS-?3'].filter((re) => new RegExp(re, 'i').test(answer))
log('\n===== LONG-SESSION COMPACTION VERIFY =====')
log(`compaction generations the secrets survived: ${generations}${newGenerations > 0 ? ` (+${newGenerations} more this turn)` : ''}`)
log(`recall: ${recalled.length}/3 secrets`)
log(`error blocks: ${errs.length}`)
const pass = generations >= 2 && recalled.length === 3 && errs.length === 0
log(
  pass
    ? '\n✅ PASSED: secrets survived a many-generation compacted session, full recall'
    : `\n❌ FAILED  answer tail: ${answer.slice(-160)}`,
)
process.exit(pass ? 0 : 1)
