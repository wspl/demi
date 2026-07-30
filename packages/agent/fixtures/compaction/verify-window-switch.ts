/**
 * Window-switch acceptance on the cached large fixture, using DeepSeek V4 Flash only.
 *
 *   STEP 1  small window → large window  ⇒ expect NO compaction
 *   STEP 2  grow replayable context on the large window
 *   STEP 3  large window → small window  ⇒ expect FORCED compaction by the pre-switch model
 *   STEP 4  switch back to large         ⇒ session must still work / recall
 *
 *   bun run fixtures:compaction:verify-switch
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
const SMALL_WINDOW = Number(process.env.COMPACTION_FIXTURE_SMALL_WINDOW ?? 8_000)
const LARGE_WINDOW = Number(process.env.COMPACTION_FIXTURE_LARGE_WINDOW ?? 400_000)
/** Optional extra grow target after the compactable-window condition is met. */
const GROW_TARGET = Number(process.env.COMPACTION_FIXTURE_GROW_TARGET ?? 0)

const fx = JSON.parse(gunzipSync(readFileSync(FIXTURE)).toString('utf8')) as {
  harnessName: string
  cwd: string
  blocks: Block[]
  builtTokens: number
}
const log = (m: string): void => process.stdout.write(`${m}\n`)

const small = await createDeepSeekFlash(SMALL_WINDOW)
const large = await createDeepSeekFlash(LARGE_WINDOW)

const environment = new BashEnvironment({
  host: new LocalHost(fx.cwd),
  shellIdFactory: () => 'xp-shell',
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
  model: small.model,
  harnessName: fx.harnessName,
}
const session = AgentSession.fromCheckpoint({ provider: small.runtime, checkpoint, runtime })
sessionRef = session

const ctx = (): number => session.transcript().estimateContextTokens()
const gens = (): number => session.transcript().blocks.filter((b) => b.type === 'compaction_boundary').length
const errCount = (): number => session.transcript().blocks.filter((b) => b.type === 'error').length
const RX = ['ZEBRA-?7', 'QUARTZ-?9', 'NIMBUS-?3']
const recallCount = (text: string): number => RX.filter((re) => new RegExp(re, 'i').test(text)).length

function responseSince(beforeLen: number): string {
  return session
    .transcript()
    .blocks.slice(beforeLen)
    .filter((b): b is Extract<Block, { type: 'text' }> => b.type === 'text' || b.type === 'response')
    .map((b) => ('text' in b ? b.text : ''))
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
      log(`   (${label} attempt ${attempt + 1} failed: ${String(e).slice(0, 70)})`)
    }
  }
  throw lastErr
}

function listTsFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === 'node_modules' || entry.name === 'dist' || entry.name.startsWith('.')) continue
    const full = join(dir, entry.name)
    if (entry.isDirectory()) listTsFiles(full, out)
    else if (entry.name.endsWith('.ts') && !entry.name.endsWith('.d.ts')) out.push(full)
  }
  return out
}

log(`loaded: ${fx.blocks.length} blocks, ctx≈${ctx()} replayable tokens, ${gens()} compaction generations`)
log(`windows: small=${SMALL_WINDOW} large=${LARGE_WINDOW} growTarget=${GROW_TARGET}`)

log(`\n── STEP 1: switch small(${SMALL_WINDOW}) → large(${LARGE_WINDOW}) — expect NO compaction`)
const g1 = gens()
session.updateModel(large.runtime, large.model)
const recall1 = recallCount(await send('step1-recall', '只回答暗号值,用「ALPHA=…, BETA=…, GAMMA=…」格式:三个暗号分别是什么?'))
const step1NoCompact = gens() === g1
log(`   compaction on larger-window switch: ${gens() > g1 ? 'YES (unexpected ✗)' : 'no ✓'}   ctx≈${ctx()}, ${gens()} generations`)
log(`   recall: ${recall1}/3`)

log(`\n── STEP 2: grow past keepRecent so shrink-switch has a real compaction window`)
const KEEP_RECENT = 4_000
const smallThreshold = Math.floor(SMALL_WINDOW * 0.8)
let growTurn = 0
const hasCompactableWindow = (): boolean => {
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
while (
  session.transcript().estimateContextTokens(SMALL_WINDOW) < smallThreshold ||
  !hasCompactableWindow()
) {
  const filler = `FILLER-${growTurn}-` + 'x'.repeat(8_000)
  await send(`grow-${growTurn}`, `忽略下列填充并只回复 ok。\n\n${filler}`)
  growTurn += 1
  if (growTurn % 2 === 0 || growTurn === 1) {
    log(
      `   grew ${growTurn} turns — ctx≈${ctx()}, estimate@small=${session.transcript().estimateContextTokens(SMALL_WINDOW)}, compactable=${hasCompactableWindow()}`,
    )
  }
  if (growTurn > 40) throw new Error('grow exceeded 40 turns without a compactable window')
}
if (GROW_TARGET > 0) {
  while (ctx() < GROW_TARGET) {
    const filler = `FILLER-extra-${growTurn}-` + 'x'.repeat(8_000)
    await send(`grow-extra-${growTurn}`, `忽略下列填充并只回复 ok。\n\n${filler}`)
    growTurn += 1
    if (growTurn > 80) break
  }
}
log(
  `   ready: ctx≈${ctx()}, estimate@small=${session.transcript().estimateContextTokens(SMALL_WINDOW)}, compactable=${hasCompactableWindow()}, turns=${growTurn}`,
)

log(`\n── STEP 3: switch large(${LARGE_WINDOW}) → small(${SMALL_WINDOW}) — expect FORCED compaction by pre-switch model`)
const g3 = gens()
const ctx3 = ctx()
const smallEstimate = session.transcript().estimateContextTokens(SMALL_WINDOW)
log(`   ctx before switch≈${ctx3} (estimate@small=${smallEstimate}, threshold@0.8=${smallThreshold})`)
session.updateModel(small.runtime, small.model)
const recall3 = recallCount(await send('step3-recall', '只回答暗号值,用「ALPHA=…, BETA=…, GAMMA=…」格式:三个暗号分别是什么?'))
const step3Forced = gens() > g3
log(`   forced compaction by pre-switch model: ${step3Forced ? `YES ✓ (+${gens() - g3} generation, now ${gens()})` : 'NO ✗'}`)
log(`   recall after shrink-window compaction: ${recall3}/3`)

log('\n── STEP 4: switch back to large — session must keep working')
session.updateModel(large.runtime, large.model)
const recall4 = recallCount(await send('step4-recall', '只回答暗号值,用「ALPHA=…, BETA=…, GAMMA=…」格式:三个暗号分别是什么?'))
log(`   recall after switch-back: ${recall4}/3`)

await session.dispose()
await small.runtime.dispose?.()
await large.runtime.dispose?.()

const errs = errCount()
const pass =
  step1NoCompact &&
  step3Forced &&
  recall1 === 3 &&
  recall3 === 3 &&
  recall4 === 3 &&
  errs === 0

log('\n===== WINDOW-SWITCH COMPACTION VERIFY =====')
log(`step 1  small→large: no compaction = ${step1NoCompact}`)
log(`step 3  large→small: forced compaction = ${step3Forced}, recall ${recall3}/3`)
log(`step 4  switch-back recall ${recall4}/3`)
log(`error blocks: ${errs}`)
log(
  pass
    ? '\n✅ PASSED: DeepSeek flash window switch — no compact up, forced compact down, full recall'
    : '\n❌ FAILED',
)
process.exit(pass ? 0 : 1)
