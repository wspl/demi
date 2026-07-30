/**
 * Loads the cached large-context fixture and checks that secrets planted before the
 * first compaction still recall under DeepSeek V4 Flash.
 *
 * After the baseline recall, forces EXTRA_GENERATIONS more compaction rounds
 * (grow → compact → recall), stopping early when recall drops below 3/3.
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
const VERIFY_CONTEXT_WINDOW = 200_000
/** Extra forced compaction generations after baseline recall (0 = baseline only). */
const EXTRA_GENERATIONS = 3
/** keepRecent while forcing extras so each compact can still find a window. */
const KEEP_RECENT = 1_000
const RECALL_PROMPT =
  '只回答暗号值,用「ALPHA=…, BETA=…, GAMMA=…」格式:我最早让你记住的三个暗号分别是什么?'
const SECRET_PATTERNS = ['ZEBRA-?7', 'QUARTZ-?9', 'NIMBUS-?3']

const fx = JSON.parse(gunzipSync(readFileSync(FIXTURE)).toString('utf8')) as {
  harnessName: string
  cwd: string
  blocks: Block[]
  builtTokens: number
}
const log = (m: string): void => process.stdout.write(`${m}\n`)

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
const session = AgentSession.fromCheckpoint(
  {
    provider,
    checkpoint: {
      transcript: { blocks: fx.blocks },
      state: {},
      phase: 'idle',
      queue: [],
      cwd: fx.cwd,
      model,
      harnessName: fx.harnessName,
    },
    runtime,
  },
  {
    compaction: {
      keepRecentTokens: EXTRA_GENERATIONS > 0 ? KEEP_RECENT : undefined,
      preflightThresholdRatio: 0.8,
    },
  },
)
sessionRef = session

const gens = (): number => session.transcript().blocks.filter((b) => b.type === 'compaction_boundary').length
const errCount = (): number => session.transcript().blocks.filter((b) => b.type === 'error').length

function responseSince(beforeLen: number): string {
  return session
    .transcript()
    .blocks.slice(beforeLen)
    .filter((b): b is Extract<Block, { type: 'text' }> => b.type === 'text')
    .map((b) => b.text)
    .join(' ')
}

async function send(label: string, text: string): Promise<string> {
  const before = session.transcript().blocks.length
  let lastErr: unknown = null
  for (let attempt = 0; attempt < 3; attempt += 1) {
    try {
      await session.send([{ type: 'text', text }])
      return responseSince(before)
    } catch (e) {
      lastErr = e
      log(`   (${label} attempt ${attempt + 1} failed: ${String(e).slice(0, 80)})`)
    }
  }
  throw lastErr
}

function recallCount(text: string): number {
  return SECRET_PATTERNS.filter((re) => new RegExp(re, 'i').test(text)).length
}

function hasCompactableWindow(): boolean {
  const window = session.transcript().findCompactionWindow(KEEP_RECENT)
  if (!window) return false
  let minCut = window.startIndex
  while (minCut < window.cutPoint) {
    const type = session.transcript().blocks[minCut]?.type
    if (type !== 'compaction_boundary' && type !== 'compaction_marker') break
    minCut += 1
  }
  return window.cutPoint > minCut
}

async function growUntilCompactable(label: string): Promise<void> {
  let turns = 0
  while (!hasCompactableWindow()) {
    const filler = `VERIFY-${label}-${turns}-` + 'x'.repeat(6_000)
    await send(`${label}-grow-${turns}`, `忽略下列填充并只回复 ok。\n\n${filler}`)
    turns += 1
    if (turns > 30) throw new Error(`${label}: could not create a compactable window`)
  }
  if (turns > 0) log(`   grew ${turns} filler turn(s) for a compactable window`)
}

const baselineGens = gens()
log(
  `loaded fixture: total≈${fx.builtTokens} tokens, ${fx.blocks.length} blocks, ${baselineGens} generations` +
    (EXTRA_GENERATIONS > 0 ? ` (extra=${EXTRA_GENERATIONS}, keepRecent=${KEEP_RECENT})` : ''),
)

type Row = { generations: number; recalled: number; answerTail: string; errors: number }
const rows: Row[] = []

log('\n── baseline recall')
{
  const answer = await send('baseline', RECALL_PROMPT)
  rows.push({
    generations: gens(),
    recalled: recallCount(answer),
    answerTail: answer.slice(-120),
    errors: errCount(),
  })
  log(`   gens=${gens()}  recall=${rows[0]!.recalled}/3  errors=${rows[0]!.errors}`)
}

let brokeAt: number | null = null
for (let extra = 1; extra <= EXTRA_GENERATIONS; extra += 1) {
  if (rows.at(-1)!.recalled < 3) {
    brokeAt = rows.at(-1)!.generations
    break
  }

  log(`\n── extra compact #${extra}/${EXTRA_GENERATIONS}`)
  const before = gens()
  await growUntilCompactable(`extra-${extra}`)
  await session.compact()
  if (gens() <= before) {
    log(`   compact did not add a generation (still ${gens()}); growing harder and retrying once`)
    await send(`extra-${extra}-force-grow`, `忽略下列填充并只回复 ok。\n\n` + `FORCE-${extra}-` + 'x'.repeat(20_000))
    await session.compact()
  }
  if (gens() <= before) {
    log(`   still no new generation after retry — stopping extra loop`)
    break
  }
  log(`   compacted: ${before} → ${gens()} generations`)

  const answer = await send(`recall-${extra}`, RECALL_PROMPT)
  const recalled = recallCount(answer)
  rows.push({ generations: gens(), recalled, answerTail: answer.slice(-120), errors: errCount() })
  log(`   gens=${gens()}  recall=${recalled}/3  errors=${errCount()}`)
  if (recalled < 3) {
    brokeAt = gens()
    break
  }
}

await session.dispose()

log('\n===== LONG-SESSION COMPACTION VERIFY =====')
if (EXTRA_GENERATIONS > 0) {
  log('generations | recall | errors')
  for (const row of rows) {
    log(`${String(row.generations).padStart(11)} | ${row.recalled}/3    | ${row.errors}`)
  }
}

const last = rows.at(-1)!
const pass =
  brokeAt === null &&
  baselineGens >= 2 &&
  last.recalled === 3 &&
  last.errors === 0 &&
  rows.every((row) => row.recalled === 3 && row.errors === 0)

if (brokeAt !== null) {
  log(`\n❌ recall dropped below 3/3 at generation ${brokeAt}`)
  const failed = rows.find((row) => row.generations === brokeAt)
  if (failed) log(`   answer tail: ${failed.answerTail}`)
  process.exit(1)
}

log(
  pass
    ? EXTRA_GENERATIONS > 0
      ? `\n✅ PASSED: full recall through ${last.generations} generations (${last.generations - baselineGens} extra forced)`
      : `\n✅ PASSED: secrets survived a many-generation compacted session, full recall (${last.generations} gens)`
    : `\n❌ FAILED  answer tail: ${last.answerTail}`,
)
process.exit(pass ? 0 : 1)
