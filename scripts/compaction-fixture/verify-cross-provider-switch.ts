/**
 * Real cross-provider switch + compaction-on-switch acceptance, on the cached large fixture.
 *
 * Exercises the exact scenario the unit tests only mock: a genuinely large context that must be
 * compacted BY THE PRE-SWITCH MODEL when switching to a smaller-window model, across a real provider
 * boundary (claude-code ↔ codex), with real thinking + tool-call history in context.
 *
 *   STEP 1  claude-code/sonnet (200k)  → codex/gpt-5.5 (272k, LARGER)  ⇒ expect NO compaction
 *   STEP 2  grow the replayable context on codex toward ~170k (codex threshold ~217k, so it holds)
 *   STEP 3  codex/gpt-5.5 (272k)      → claude-code/sonnet (200k, SMALLER) ⇒ expect FORCED compaction
 *           run by the pre-switch CODEX model (it can still load 170k to summarize), then claude continues
 *   STEP 4  switch back to codex ⇒ session must still work ("切回来要能继续工作")
 *
 * Passes only if: step 1 does not compact, step 3 forces a compaction generation by codex and the
 * secrets still recall, step 4 still recalls, and no error blocks appear. Uses REAL default
 * compaction thresholds. Calls BOTH real providers — needs `claude` + `~/.codex` auth, costs a few $.
 *
 *   bun run scripts/compaction-fixture/verify-cross-provider-switch.ts
 */
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'
import { gunzipSync } from 'node:zlib'
import type { Block, ModelSelection } from '../../packages/core/src/index'
import { ClaudeCodeProvider } from '../../packages/provider-claude-code/src/provider'
import { CodexProvider } from '../../packages/provider-codex/src/provider'
import { AgentSession } from '../../packages/agent/src/index'
import { BashEnvironment, createShellSessionTools } from '../../packages/shell/src/index'
import { LocalHost } from '../../packages/host-local/src/index'

const REPO = join(import.meta.dir, '../..')
const FIXTURE = join(import.meta.dir, 'large-context-fixture.json.gz')
const GROW_TARGET = 170_000 // above claude's 160k threshold, below codex's ~217k threshold
process.env.DEMI_CLAUDE_WIRE_LOG = '0'

const fx = JSON.parse(gunzipSync(readFileSync(FIXTURE)).toString('utf8')) as {
  harnessName: string
  cwd: string
  model: ModelSelection
  blocks: Block[]
  builtTokens: number
  generations: number
}
const log = (m: string): void => process.stdout.write(`${m}\n`)

const claudeModel = fx.model // claude-code/sonnet, 200k
const codexModel: ModelSelection = {
  providerId: 'codex',
  model: { id: 'gpt-5.5', name: 'GPT-5.5', contextWindow: 272000, inputLimit: null, thinking: [], acceptedExtensions: [] },
  thinking: null,
}

const environment = new BashEnvironment({ host: new LocalHost(fx.cwd), shellIdFactory: () => 'xp-shell', initialEnv: { PATH: process.env.PATH ?? '' } })
const runtime = {
  harnessName: fx.harnessName,
  initialState: () => ({}),
  systemPrompt: () => 'You are a careful coding assistant. Remember any secrets the user told you verbatim.',
  tools: () => createShellSessionTools(environment),
}
const snapshot = { transcript: { blocks: fx.blocks }, state: {}, phase: 'idle' as const, queue: [], cwd: fx.cwd, model: claudeModel, harnessName: fx.harnessName }
const session = AgentSession.fromSnapshot({ provider: new ClaudeCodeProvider({}), snapshot, runtime }) // real default compaction

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
const files = listTsFiles(join(REPO, 'packages'))
  .map((f) => ({ f: relative(REPO, f), size: statSync(f).size }))
  .sort((a, b) => b.size - a.size)
  .map((x) => x.f)

log(`loaded: ${fx.blocks.length} blocks, ctx≈${ctx()} replayable tokens, ${gens()} compaction generations`)

// ── STEP 1: claude(200k) → codex(272k) — larger window, expect NO compaction ──
log('\n── STEP 1: switch claude-code/sonnet(200k) → codex/gpt-5.5(272k) — larger window, expect NO compaction')
const g1 = gens()
session.updateModel(new CodexProvider({}), codexModel)
const r1 = await send('step1', '我们刚把模型从 Claude 切到了 GPT-5(codex)。先确认你能接上之前的上下文:用一句话说明你现在还记得我最早让你记住的几个暗号(不用说具体值)。')
const step1NoCompact = gens() === g1
log(`   codex replied: ${r1.slice(0, 120).replace(/\s+/g, ' ')}`)
log(`   compaction on larger-window switch: ${gens() > g1 ? 'YES (unexpected ✗)' : 'no ✓'}   ctx≈${ctx()}, ${gens()} generations`)

// ── STEP 2: grow the replayable context on codex toward ~170k ──
log(`\n── STEP 2: grow context on codex toward ~${GROW_TARGET} tokens (codex threshold ~217k, so codex holds it)`)
let fi = 0
while (ctx() < GROW_TARGET && fi < files.length) {
  let buf = ''
  const used: string[] = []
  while (buf.length < 110_000 && fi < files.length) {
    try {
      buf += `\n\n// ===== ${files[fi]} =====\n${readFileSync(join(REPO, files[fi]), 'utf8')}`
      used.push(files[fi])
    } catch {
      /* skip unreadable */
    }
    fi += 1
  }
  if (!buf) break
  await send('grow', `继续阅读以下 demi 源码文件,每个文件用一句话说明它的职责:\n\`\`\`ts${buf}\n\`\`\``)
  log(`   +${used.length} files — ctx≈${ctx()} tokens, ${gens()} generations (on codex)`)
}
log(`   grown to ctx≈${ctx()} tokens, ${gens()} generations`)

// ── STEP 3: codex(272k) → claude(200k) — smaller window, expect FORCED compaction BY CODEX ──
log('\n── STEP 3: switch codex/gpt-5.5(272k) → claude-code/sonnet(200k) — smaller window, expect FORCED compaction by pre-switch codex')
const g3 = gens()
const ctx3 = ctx()
session.updateModel(new ClaudeCodeProvider({}), claudeModel)
const r3 = await send('step3', '只回答暗号值,用「ALPHA=…, BETA=…, GAMMA=…」格式:我在整段对话最开始让你逐字记住的三个暗号分别是什么?')
const step3Forced = gens() > g3
const recall3 = recallCount(r3)
log(`   ctx before switch≈${ctx3} (> 160k claude threshold ⇒ must compact before claude can load it)`)
log(`   forced compaction by pre-switch codex: ${step3Forced ? `YES ✓ (+${gens() - g3} generation, now ${gens()})` : 'NO ✗'}`)
log(`   claude replied: ${r3.slice(0, 160).replace(/\s+/g, ' ')}`)
log(`   recall after cross-provider compaction: ${recall3}/3`)

// ── STEP 4: switch back to codex — session must still work ──
log('\n── STEP 4: switch back claude-code → codex — verify the session still works after switching back')
session.updateModel(new CodexProvider({}), codexModel)
const r4 = await send('step4', '最后再确认一次:那三个暗号分别是什么?用「ALPHA=…, BETA=…, GAMMA=…」格式回答。')
const recall4 = recallCount(r4)
log(`   codex replied: ${r4.slice(0, 160).replace(/\s+/g, ' ')}`)
log(`   recall after switching back: ${recall4}/3`)

const errs = errCount()
await session.dispose()

log('\n===== CROSS-PROVIDER SWITCH + COMPACTION-ON-SWITCH VERIFY =====')
log(`step 1  claude→codex (larger 272k):  no compaction = ${step1NoCompact}`)
log(`step 3  codex→claude (smaller 200k): forced compaction by codex = ${step3Forced}, recall ${recall3}/3`)
log(`step 4  switch back to codex:        recall ${recall4}/3`)
log(`error blocks: ${errs}`)
const pass = step1NoCompact && step3Forced && recall3 === 3 && recall4 === 3 && errs === 0
log(
  pass
    ? '\n✅ PASSED: real claude-code↔codex switch — no compaction switching up, forced compaction-on-switch by the pre-switch model switching down, full recall both directions, session keeps working after switching back'
    : '\n❌ FAILED — see steps above',
)
process.exit(pass ? 0 : 1)
