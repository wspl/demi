import { homedir } from 'node:os'
import { join } from 'node:path'
import process from 'node:process'
import type { Provider } from '@demicodes/provider'
import { createAnthropicApiProvider } from '@demicodes/provider-anthropic-api'
import { createGoogleProvider } from '@demicodes/provider-google'
import { createOpenAIApiProvider } from '@demicodes/provider-openai-api'
import { createBackend } from './backend'

/**
 * Operator-assembled entry (M1): providers come from environment API keys,
 * exactly like the dev product. The credential vault replaces this in M3.
 */
function assembleProviders(): Provider[] {
  const providers: Provider[] = []
  if (process.env.ANTHROPIC_API_KEY) {
    providers.push(createAnthropicApiProvider({ apiKey: () => process.env.ANTHROPIC_API_KEY }))
  }
  if (process.env.OPENAI_API_KEY) {
    providers.push(createOpenAIApiProvider({ apiKey: () => process.env.OPENAI_API_KEY }))
  }
  const googleKey = process.env.GEMINI_API_KEY ?? process.env.GOOGLE_API_KEY
  if (googleKey) providers.push(createGoogleProvider({ apiKey: () => googleKey }))
  return providers
}

async function main(): Promise<void> {
  const dataDir = process.env.DEMI_BACKEND_DATA ?? join(homedir(), '.demi', 'backend')
  const port = Number(process.env.DEMI_BACKEND_PORT ?? 3271)
  const providers = assembleProviders()
  if (providers.length === 0) {
    console.error('No providers configured: set ANTHROPIC_API_KEY / OPENAI_API_KEY / GEMINI_API_KEY.')
    process.exit(2)
  }
  const backend = await createBackend({ dataDir, providers, port })
  console.log(`demi-backend listening on ${backend.url} (data: ${dataDir})`)

  const shutdown = () => {
    void backend.close().then(() => process.exit(0))
  }
  process.on('SIGINT', shutdown)
  process.on('SIGTERM', shutdown)
}

void main()
