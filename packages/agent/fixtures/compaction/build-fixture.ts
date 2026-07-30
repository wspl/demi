/**
 * Builds a genuinely large, real long-session fixture (gzipped) next to this file.
 *
 * Plants three secrets, then grows history under DeepSeek V4 Flash until the session
 * has compacted a target number of generations. Prefer refreshing only when the
 * committed fixture is intentionally replaced — verifiers reuse the existing blob.
 *
 *   bun run fixtures:compaction:build [targetGenerations]
 *
 * Requires `DEEPSEEK_API_KEY` in the repo-root `.env`.
 */
import { writeFileSync, readdirSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'
import { gzipSync } from 'node:zlib'
import { AgentSession, createStandardAgentTools } from '../../src/index'
import { BashEnvironment } from '@demicodes/shell'
import { LocalHost } from '@demicodes/host-local'
import { createDeepSeekFlash } from './deepseek'

const REPO = join(import.meta.dir, '../../../..')
const FIXTURE = join(import.meta.dir, 'large-context-fixture.json.gz')
const TARGET_GENERATIONS = Number(process.argv[2] ?? 4)
const HARD_TOKEN_CAP = 2_000_000
/** Small registered window so flash actually compacts while building. */
const BUILD_CONTEXT_WINDOW = Number(process.env.COMPACTION_FIXTURE_BUILD_WINDOW ?? 100_000)

const { runtime: provider, model } = await createDeepSeekFlash(BUILD_CONTEXT_WINDOW)
const environment = new BashEnvironment({
  host: new LocalHost(REPO),
  shellIdFactory: () => 'fx-shell',
  initialEnv: { PATH: process.env.PATH ?? '' },
})
let sessionRef: AgentSession<Record<string, never>> | null = null
const runtime = {
  harnessName: 'fixture',
  initialState: () => ({}),
  systemPrompt: () => 'You are a careful coding assistant. Remember any secrets the user tells you verbatim, forever.',
  tools: () =>
    createStandardAgentTools({
      environment,
      scheduleYield: (_ctx, durationMs) => {
        if (!sessionRef) throw new Error('fixture session is not ready for yield scheduling')
        return sessionRef.scheduleYieldWakeup(durationMs)
      },
    }),
}
const session = new AgentSession(
  { provider, model, cwd: REPO, runtime },
  { compaction: { keepRecentTokens: 6000, preflightThresholdRatio: 0.7 } },
)
sessionRef = session

const log = (m: string): void => process.stdout.write(`${m}\n`)
const totalTokens = (): number => Math.round(JSON.stringify(session.transcript().blocks).length / 4)
const boundaries = (): number => session.transcript().blocks.filter((b) => b.type === 'compaction_boundary').length

function listTsFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === 'node_modules' || entry.name === 'dist' || entry.name.startsWith('.')) continue
    const full = join(dir, entry.name)
    if (entry.isDirectory()) listTsFiles(full, out)
    else if (entry.name.endsWith('.ts') && !entry.name.endsWith('.d.ts')) out.push(full)
  }
  return out
}
const files = listTsFiles(join(REPO, 'packages'))
  .map((f) => ({ f: relative(REPO, f), size: statSync(f).size }))
  .sort((a, b) => b.size - a.size)
  .map((x) => x.f)

log(`plant secrets — target ${TARGET_GENERATIONS} generations on deepseek-v4-flash (window=${BUILD_CONTEXT_WINDOW})`)
await session.send([
  {
    type: 'text',
    text: '请逐字记住三个暗号,整段对话里我会反复考你,无论压缩多少次都要记住:SECRET_ALPHA=ZEBRA-7、SECRET_BETA=QUARTZ-9、SECRET_GAMMA=NIMBUS-3。只回复「已记住」。',
  },
])

let gens = 0
for (let i = 0; i < files.length && boundaries() < TARGET_GENERATIONS && totalTokens() < HARD_TOKEN_CAP; i += 1) {
  try {
    await session.send([{ type: 'text', text: `运行 \`cat ${files[i]}\` 读取这个文件,然后用一句话说明它的职责。` }])
  } catch (e) {
    log(`  (turn ${i} failed: ${String(e).slice(0, 80)})`)
  }
  if (boundaries() > gens) {
    gens = boundaries()
    log(`  ── compaction generation ${gens} reached after ${i + 1} files (total≈${totalTokens()} tokens, ${session.transcript().blocks.length} blocks)`)
  } else if (i % 3 === 0) {
    log(`  read ${i + 1} files — total≈${totalTokens()} tokens, ${boundaries()} generations, ${session.transcript().blocks.length} blocks`)
  }
}

const rawBlocks = session.transcript().blocks
const blocks = rawBlocks.filter((b) => b.type !== 'error')
const stripped = rawBlocks.length - blocks.length
if (stripped > 0) log(`  stripped ${stripped} transient error block(s) from the saved transcript`)
const builtTokens = Math.round(JSON.stringify(blocks).length / 4)
const payload = JSON.stringify({
  harnessName: 'fixture',
  cwd: REPO,
  model,
  blocks,
  builtTokens,
  generations: boundaries(),
})
writeFileSync(FIXTURE, gzipSync(payload))
await session.dispose()

const has = (s: string): string => (JSON.stringify(blocks).includes(s) ? '✓' : '✗')
log(`\n✅ fixture saved (gzipped): ${FIXTURE}`)
log(`   total≈${totalTokens()} tokens  blocks=${blocks.length}  compaction generations=${boundaries()}`)
log(`   block types: ${[...new Set(blocks.map((b) => b.type))].join(', ')}`)
log(`   secrets still present in full transcript: ALPHA${has('ZEBRA-7')} BETA${has('QUARTZ-9')} GAMMA${has('NIMBUS-3')}`)
