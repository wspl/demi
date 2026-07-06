import { errorMessage } from '@demicodes/utils'
import { mkdir } from 'node:fs/promises'
import process from 'node:process'
import { createInterface } from 'node:readline/promises'
import { AgentClient, AgentServer } from '@demicodes/agent'
import type { ProviderSelection } from '@demicodes/provider'
import { createCodingAgentHarness } from '@demicodes/coding-agent'
import { LocalHost } from '@demicodes/host-local'
import { parseArgs, type ReplOptions } from './options'
import { color, writeEventLine, writeLine, writeMetaLine } from './output'
import { attachRenderer, createRenderer } from './render'
import { runInputLoop } from './input-loop'
import { createReplProviders, printCodexAuthStatus, providerFor, resolveReplModel, type ResolvedReplModel } from './model'

export { parseArgs, printUsage, type ReplOptions } from './options'
export { color, writeEventLine, writeLine, writeLineTo, writeMetaLine, type ReplOutput, type Tone } from './output'
export {
  attachRenderer,
  createRenderer,
  finishStream,
  renderEvent,
  type RenderState,
  type ReplEventSource,
} from './render'
export { handleCommand, helpText, runInputLoop, type ReplInputLoop } from './input-loop'
export {
  createReplProviders,
  printCodexAuthStatus,
  providerDisplayName,
  providerFor,
  resolveReplModel,
  type ResolvedReplModel,
} from './model'

async function main(): Promise<void> {
  const options = parseArgs(process.argv.slice(2))
  await mkdir(options.cwd, { recursive: true })

  const host = new LocalHost(options.cwd)
  const harness = createCodingAgentHarness({ host })

  const providers = createReplProviders(options)
  const activeProvider = providerFor(providers, options.provider)
  const model = await resolveReplModel(activeProvider, options)

  printBanner(options, model)
  if (options.provider === 'codex') await printCodexAuthStatus(options)

  const server = new AgentServer({
    agent: harness,
    providers,
    shell: {
      initialEnv: { PATH: process.env.PATH ?? '' },
    },
  })
  const client = server.client()
  const renderer = createRenderer()
  attachRenderer(client, renderer)

  const providerSelection: ProviderSelection = {
    providerId: options.provider,
    model: model.selection,
  }

  await client.open(providerSelection, options.cwd, globalThis.crypto.randomUUID())
  writeEventLine(process.stdout, 'state', 'session opened; type /help for commands, /exit to quit', 'dim')

  const rl = createInterface({ input: process.stdin, output: process.stdout })
  let closing = false
  process.on('SIGINT', () => {
    if (closing) return
    if (renderer.phase && renderer.phase !== 'idle') {
      writeEventLine(process.stdout, 'state', 'aborting active turn', 'yellow')
      void client.abort().catch((error) => writeEventLine(process.stdout, 'error', `abort failed: ${errorMessage(error)}`, 'red'))
      return
    }
    closing = true
    writeEventLine(process.stdout, 'state', 'closing', 'dim')
    void cleanup(rl, client, server).finally(() => process.exit(0))
  })

  try {
    const prompt = process.stdin.isTTY ? '\ndemi> ' : ''
    await runInputLoop({
      ask: () => rl.question(color(prompt, 'bold')),
      client,
      renderer,
      output: process.stdout,
      shouldContinue: () => !closing,
    })
  } finally {
    await cleanup(rl, client, server)
  }
}

async function cleanup(
  rl: ReturnType<typeof createInterface>,
  client: AgentClient,
  server: AgentServer,
): Promise<void> {
  rl.close()
  try {
    await client.close()
  } finally {
    await server.close()
  }
}

function printBanner(options: ReplOptions, model: ResolvedReplModel): void {
  writeLine(color('Demi REPL', 'bold'))
  writeLine(color('interactive agent session', 'dim'))
  writeMetaLine('provider', options.provider)
  writeMetaLine('cwd', options.cwd)
  writeMetaLine('model', model.selection.model.id)
  writeMetaLine('thinking', options.thinkingEffort ?? 'not requested')
  if (options.serviceTierId) writeMetaLine('tier', options.serviceTierId)
  if (options.provider === 'openai') writeMetaLine('openai wire api', options.openAIWireApi)
  if (options.provider === 'codex') writeMetaLine('transport', options.transport)
  for (const warning of model.warnings) writeEventLine(process.stdout, 'warning', warning, 'yellow')
}

if (import.meta.main) {
  main().catch((error) => {
    process.stderr.write(`fatal: ${errorMessage(error)}\n`)
    process.exit(1)
  })
}
