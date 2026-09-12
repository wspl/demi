// A development backend for the web front end: a fresh data directory, the
// master account set up, and one scripted provider whose model echoes the
// message back. A message containing "fail" ends its turn with a provider
// error, so the transcript's error record and its Retry can be exercised
// without a real model. Run it with `bun run --conditions development
// packages/backend/src/dev.ts`; the web dev server proxies to its URL through
// `DEMI_BACKEND_URL`.
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import process from 'node:process'
import { defineProvider, type AgentProvider, type InferenceRequest, type ProviderEvent } from '@demicodes/provider'
import { events } from '@demicodes/provider/testing'
import { createBackend } from './backend'
import { SESSION_COOKIE } from './http/cookies'

const DEV_USER = { email: 'dev@example.test', password: 'dev-pass-1234' }

function lastUserText(request: InferenceRequest): string {
  const item = [...request.items].reverse().find((entry) => entry.type === 'user_message')
  if (!item || item.type !== 'user_message') {
    return ''
  }
  return item.content
    .map((part) => (part.type === 'text' ? part.text : ''))
    .join(' ')
    .trim()
}

const echoProvider: AgentProvider = {
  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    const text = lastUserText(request)
    // Long enough for the running state to be seen.
    await new Promise((resolve) => setTimeout(resolve, 1500))
    if (/\bfail\b/i.test(text)) {
      throw new Error(`Scripted provider failure for "${text}"`)
    }
    yield events.text(`Echo: ${text || '(no text)'}`)
    yield events.response({ inputTokens: 12, outputTokens: 4 })
  },
  clone() {
    return echoProvider
  },
}

async function main(): Promise<void> {
  const port = Number(process.env.DEMI_BACKEND_PORT ?? 3299)
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-dev-backend-'))
  const backend = await createBackend({
    dataDir,
    port,
    mode: 'shared',
    providerTypes: {
      echo: {
        credential: 'api_key',
        create: ({ providerId, label }) =>
          defineProvider({ id: providerId, displayName: label, createRuntime: () => echoProvider }),
      },
    },
  })
  const setup = await fetch(`${backend.url}/api/setup`, {
    method: 'POST',
    body: JSON.stringify(DEV_USER),
    headers: { 'content-type': 'application/json' },
  })
  if (setup.status !== 201) {
    throw new Error(`setup failed: HTTP ${setup.status} ${await setup.text()}`)
  }
  const cookie = setup.headers.get('set-cookie')?.match(new RegExp(`${SESSION_COOKIE}=([^;]+)`))?.[1]
  if (!cookie) {
    throw new Error('setup returned no session cookie')
  }
  const provider = await fetch(`${backend.url}/api/providers`, {
    method: 'POST',
    body: JSON.stringify({
      providerType: 'echo',
      label: 'Echo',
      apiKey: 'dev-key',
      models: [{
        id: 'echo',
        displayName: 'Echo',
        contextWindow: 100_000,
        outputLimit: null,
        thinkingEfforts: [],
        acceptedExtensions: null,
        fastTier: null,
      }],
    }),
    headers: { 'content-type': 'application/json', cookie: `${SESSION_COOKIE}=${cookie}` },
  })
  if (provider.status !== 201 && provider.status !== 200) {
    throw new Error(`provider setup failed: HTTP ${provider.status} ${await provider.text()}`)
  }
  console.log(`demi dev backend listening on ${backend.url} (data: ${dataDir})`)
  console.log(`sign in as ${DEV_USER.email} / ${DEV_USER.password}, or send the cookie ${SESSION_COOKIE}=${cookie}`)

  const shutdown = () => {
    void backend.close().then(() => process.exit(0))
  }
  process.on('SIGINT', shutdown)
  process.on('SIGTERM', shutdown)
}

void main()
