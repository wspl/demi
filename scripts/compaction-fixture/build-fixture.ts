/**
 * Builds a real, large-context conversation once (real model, real tool calls, planted secrets)
 * and caches its transcript to `.test-cache/large-context-fixture.json`. Reuse it with
 * `verify-compaction.ts` to exercise compaction / model-switch scenarios cheaply, without
 * regenerating the context each run.
 *
 *   bun run scripts/compaction-fixture/build-fixture.ts
 *
 * Calls the real Claude Code provider — needs `claude` auth and costs a little.
 */
import { writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { ClaudeCodeProvider } from '../../packages/provider-claude-code/src/provider'
import { AgentSession } from '../../packages/agent/src/index'
import { BashEnvironment, createShellSessionTools } from '../../packages/shell/src/index'
import { LocalHost } from '../../packages/host-local/src/index'

const REPO = join(import.meta.dir, '../..')
const FIXTURE = join(import.meta.dir, 'large-context-fixture.json')
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
  systemPrompt: () => 'You are a careful coding assistant. Remember any secrets the user tells you verbatim.',
  tools: () => createShellSessionTools(environment),
}
const session = new AgentSession({ provider, model, cwd: REPO, runtime })

const tokens = (): number => session.transcript().estimateContextTokens()
const log = (m: string): void => process.stdout.write(`${m}\n`)

log('plant secrets')
await session.send([
  {
    type: 'text',
    text: '请逐字记住三个暗号,后面会考你:SECRET_ALPHA=ZEBRA-7、SECRET_BETA=QUARTZ-9、SECRET_GAMMA=NIMBUS-3。只回复「已记住」。',
  },
])

const reads = [
  'cat packages/agent/src/session.ts | sed -n "1,260p"',
  'cat packages/provider-claude-code/src/provider.ts | sed -n "1,220p"',
  'cat packages/agent/src/transcript.ts | sed -n "1,200p"',
  'cat packages/provider-claude-code/src/jsonl.ts | sed -n "1,170p"',
]
for (let i = 0; i < reads.length && tokens() < 16000; i += 1) {
  log(`read+explain #${i + 1} (tokens≈${tokens()})`)
  await session.send([{ type: 'text', text: `用 shell 运行:\`${reads[i]}\`,然后用 4-6 句话解释这段代码的职责与关键逻辑。` }])
}

const blocks = session.transcript().blocks
writeFileSync(FIXTURE, `${JSON.stringify({ harnessName: 'fixture', cwd: REPO, model, blocks, builtTokens: tokens() }, null, 2)}\n`)
await session.dispose()

log(`\n✅ fixture saved: ${FIXTURE}`)
log(`   estimateContextTokens≈${tokens()}  blocks=${blocks.length}  types=${[...new Set(blocks.map((b) => b.type))].join(', ')}`)
const has = (s: string): string => (JSON.stringify(blocks).includes(s) ? '✓' : '✗')
log(`   secrets present: ALPHA${has('ZEBRA-7')} BETA${has('QUARTZ-9')} GAMMA${has('NIMBUS-3')}`)
