import process from 'node:process'
import type { Provider } from '@demi/provider'
import { createAnthropicApiProvider } from '@demi/provider-anthropic-api'
import { createClaudeCodeProvider, resolveWireLogDir } from '@demi/provider-claude-code'
import { createCodexProvider } from '@demi/provider-codex'
import { createOpenAIApiProvider } from '@demi/provider-openai-api'
import { parseServerOptions } from './server-options'
import { startWebServer } from './serve'

const options = parseServerOptions(process.argv.slice(2))
const claudeCodeProvider = createClaudeCodeProvider({ claudePath: options.claudePath })
const codexProvider = createCodexProvider({
  codexHome: options.codexHome,
  baseUrl: options.provider === 'codex' ? options.baseUrl : undefined,
  transport: options.transport,
})
const openAIProvider = createOpenAIApiProvider({
  baseUrl: options.provider === 'openai' ? options.baseUrl : undefined,
  wireApi: options.openAIWireApi,
})
const anthropicProvider = createAnthropicApiProvider({
  baseUrl: options.provider === 'anthropic' ? options.baseUrl : undefined,
})
const providers: Provider[] = orderProviders(
  [claudeCodeProvider, codexProvider, openAIProvider, anthropicProvider],
  options.provider,
)

const handle = startWebServer(providers, options)
process.stdout.write(`demi web listening on ${handle.url} — cwd ${options.cwd}, provider ${options.provider}\n`)
const wireLogDir = resolveWireLogDir()
process.stdout.write(`claude wire log: ${wireLogDir ?? '(disabled)'}\n`)

function orderProviders(providers: Provider[], selectedProviderId: string): Provider[] {
  const selected = providers.find((provider) => provider.id === selectedProviderId)
  return selected ? [selected, ...providers.filter((provider) => provider.id !== selectedProviderId)] : providers
}
