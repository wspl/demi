/**
 * Builds a genuinely large, real long-session fixture and caches it (gzipped) next to this file.
 *
 * A single model request can never exceed the model's context window (~200k), so a "large context"
 * is a session that has already compacted several times: the cumulative transcript grows far past
 * the window while each request stays under it. `Transcript.insertCompactionBoundary` keeps the old
 * blocks (splices a boundary in, deleting nothing) and `replayableBlocks()` slices from the last
 * boundary — so the saved transcript can be huge even though each replayed request is window-bounded.
 *
 * The build reads/explains real source files under default compaction until the session has compacted
 * a target number of GENERATIONS (default 4) — targeting generations, not a token count, so it's
 * robust regardless of how the size proxy maps to the compactor's real token estimate. Three secrets
 * are planted up front so the verifier can check they survive being re-summarized every generation.
 *
 *   bun run scripts/compaction-fixture/build-fixture.ts [targetGenerations]
 *
 * Calls the real Claude Code provider — needs `claude` auth, and a long build costs a few dollars.
 */
import { writeFileSync } from 'node:fs'
import { readdirSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'
import { gzipSync } from 'node:zlib'
import { ClaudeCodeProvider } from '../../packages/provider-claude-code/src/provider'
import { AgentSession } from '../../packages/agent/src/index'
import { BashEnvironment, createShellSessionTools } from '../../packages/shell/src/index'
import { LocalHost } from '../../packages/host-local/src/index'

const REPO = join(import.meta.dir, '../..')
const FIXTURE = join(import.meta.dir, 'large-context-fixture.json.gz')
const TARGET_GENERATIONS = Number(process.argv[2] ?? 4)
const HARD_TOKEN_CAP = 2_000_000
process.env.DEMI_CLAUDE_WIRE_LOG = '0'

const model = {
  providerId: 'claude-code',
  model: { id: 'sonnet', name: 'Sonnet 4.6', contextWindow: 200000, inputLimit: null, thinking: [], acceptedExtensions: [] },
  thinking: null,
}
const provider = new ClaudeCodeProvider({})
const environment = new BashEnvironment({ host: new LocalHost(REPO), shellIdFactory: () => 'fx-shell', initialEnv: { PATH: process.env.PATH ?? '' } })
const runtime = {
  harnessName: 'fixture',
  initialState: () => ({}),
  systemPrompt: () => 'You are a careful coding assistant. Remember any secrets the user tells you verbatim, forever.',
  tools: () => createShellSessionTools(environment),
}
// Default-ish compaction so the session compacts for real as it grows past the window.
const session = new AgentSession({ provider, model, cwd: REPO, runtime }, { compaction: { keepRecentTokens: 6000, preflightThresholdRatio: 0.7 } })

const log = (m: string): void => process.stdout.write(`${m}\n`)
// Size proxy over the FULL transcript (replayableBlocks is window-bounded; we want the whole thing).
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

log(`plant secrets — target ${TARGET_GENERATIONS} compaction generations`)
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

// Strip any transient `error` blocks left by build-time connection drops — they're not part of the
// intended large-context history and would otherwise pollute a reusable fixture. (Cold-start leaves
// no orphaned tool calls, so this is a clean removal.)
const rawBlocks = session.transcript().blocks
const blocks = rawBlocks.filter((b) => b.type !== 'error')
const stripped = rawBlocks.length - blocks.length
if (stripped > 0) log(`  stripped ${stripped} transient error block(s) from the saved transcript`)
const builtTokens = Math.round(JSON.stringify(blocks).length / 4)
const payload = JSON.stringify({ harnessName: 'fixture', cwd: REPO, model, blocks, builtTokens, generations: boundaries() })
writeFileSync(FIXTURE, gzipSync(payload))
await session.dispose()

const has = (s: string): string => (JSON.stringify(blocks).includes(s) ? '✓' : '✗')
log(`\n✅ fixture saved (gzipped): ${FIXTURE}`)
log(`   total≈${totalTokens()} tokens  blocks=${blocks.length}  compaction generations=${boundaries()}`)
log(`   block types: ${[...new Set(blocks.map((b) => b.type))].join(', ')}`)
log(`   secrets still present in full transcript: ALPHA${has('ZEBRA-7')} BETA${has('QUARTZ-9')} GAMMA${has('NIMBUS-3')}`)
