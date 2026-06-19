import { expect, test } from 'bun:test'
import { randomUUID } from 'node:crypto'
import type { ModelSelection, TokenUsage } from '@demi/core'
import { AgentSession, type AgentHarnessRuntime } from '@demi/agent'
import type { InferenceRequest, ProviderEvent } from '@demi/provider'
import { BashEnvironment, createShellSessionTools } from '@demi/shell'
import { LocalHost } from '@demi/shell/local-host'
import { CodexProvider, FileCodexAuthStore } from '../index'

const e2e = process.env.DEMI_CODEX_E2E === '1' ? test : test.skip
const cacheE2e = process.env.DEMI_CODEX_CACHE_E2E === '1' ? test : test.skip
const thinkingE2e = process.env.DEMI_CODEX_THINKING_E2E === '1' ? test : test.skip
const toolE2e = process.env.DEMI_CODEX_TOOL_E2E === '1' ? test : test.skip
const modelId = process.env.DEMI_CODEX_MODEL ?? 'gpt-5.4'
const transport = parseTransport(process.env.DEMI_CODEX_TRANSPORT)

e2e('CodexProvider can stream a minimal response from real Codex auth', async () => {
  await expectCodexAuthAvailable()
  const events = await runCodexRequest({
    sessionId: `codex-real-e2e-${randomUUID()}`,
    systemPrompt: 'Follow exact-output requests.',
    items: [{ type: 'user_message', content: [{ type: 'text', text: 'Reply with exactly DEMI_CODEX_OK.' }] }],
    tools: [],
    thinking: null,
  })

  expectNoProviderErrors(events)
  expect(textFrom(events)).toContain('DEMI_CODEX_OK')
  expect(events.some((event) => event.type === 'response')).toBe(true)
})

thinkingE2e(
  'CodexProvider streams real medium thinking and usage',
  async () => {
    await expectCodexAuthAvailable()
    const events = await runCodexRequest({
      sessionId: `codex-thinking-e2e-${randomUUID()}`,
      systemPrompt: [
        'Use the requested reasoning effort.',
        'After reasoning, output DEMI_CODEX_THINKING_OK exactly.',
      ].join('\n'),
      items: [
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'Summarize this smoke test in your reasoning, then output only the marker.' }],
        },
      ],
      tools: [],
      thinking: { type: 'effort', effort: 'medium', summary: null },
    })

    expectNoProviderErrors(events)
    expect(textFrom(events)).toContain('DEMI_CODEX_THINKING_OK')
    expect(events.some(isVisibleThinkingEvent)).toBe(true)
    const usage = usageFrom(events)
    expect(usage.inputTokens).toBeGreaterThan(0)
    expect(usage.outputTokens).toBeGreaterThan(0)
  },
  120_000,
)

cacheE2e('CodexProvider reports provider cache usage on repeated stable-prefix requests', async () => {
  await expectCodexAuthAvailable()
  const cacheKey = `demi-codex-cache-${randomUUID()}`
  const sessionId = `codex-cache-e2e-${randomUUID()}`
  const systemPrompt = [
    `Cache smoke key: ${cacheKey}`,
    'You are testing a stable provider prefix. '.repeat(1200),
    'When asked for the marker, output DEMI_CODEX_CACHE_OK exactly.',
  ].join('\n')
  const firstEvents = await runCodexRequest({
    sessionId,
    systemPrompt,
    items: [{ type: 'user_message', content: [{ type: 'text', text: 'Output DEMI_CODEX_CACHE_OK exactly.' }] }],
    tools: [],
    thinking: null,
  })
  const secondEvents = await runCodexRequest({
    sessionId,
    systemPrompt,
    items: [{ type: 'user_message', content: [{ type: 'text', text: 'Output DEMI_CODEX_CACHE_OK exactly.' }] }],
    tools: [],
    thinking: null,
  })
  const first = usageFrom(firstEvents)
  const second = usageFrom(secondEvents)

  expectNoProviderErrors(firstEvents)
  expectNoProviderErrors(secondEvents)
  expect(textFrom(firstEvents)).toContain('DEMI_CODEX_CACHE_OK')
  expect(textFrom(secondEvents)).toContain('DEMI_CODEX_CACHE_OK')
  expect(first.inputTokens).toBeGreaterThan(1000)
  expect(second.cacheReadTokens).toBeGreaterThan(0)
  expect(second.inputTokens).toBeLessThan(first.inputTokens)
})

toolE2e('CodexProvider drives a real AgentSession shell tool roundtrip', async () => {
  await expectCodexAuthAvailable()
  const provider = new CodexProvider({ transport, maxRetries: 1 })
  const environment = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => `codex-tool-e2e-shell-${randomUUID()}`,
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  const runtime: AgentHarnessRuntime<Record<string, never>> = {
    harnessName: 'codex-real-tool-e2e',
    initialState: () => ({}),
    systemPrompt: () => [
      'You have a shell_exec tool.',
      'For this smoke test, call shell_exec exactly once with `printf DEMI_CODEX_TOOL_OK`, then answer with DEMI_CODEX_TOOL_DONE.',
    ].join('\n'),
    tools: () => createShellSessionTools(environment),
  }
  const model: ModelSelection = {
    providerId: 'codex',
    model: {
      id: modelId,
      name: modelId,
      contextWindow: 200_000,
      inputLimit: null,
      thinking: [],
      acceptedExtensions: [],
    },
    thinking: { type: 'effort', effort: 'medium', summary: null },
  }
  const session = new AgentSession({ provider, model, cwd: process.cwd(), runtime }, { agentSessionId: `codex-tool-e2e-${randomUUID()}` })

  await session.send([{ type: 'text', text: 'Run the required shell command and then report the marker.' }])

  const toolBlock = session.transcript().blocks.find((block) => block.type === 'tool_call')
  expect(toolBlock).toBeDefined()
  expect(toolBlock?.type === 'tool_call' ? toolBlock.output.some((item) => item.type === 'text' && item.text.includes('DEMI_CODEX_TOOL_OK')) : false).toBe(true)
  const transcriptText = session.transcript().blocks
    .filter((block) => block.type === 'text')
    .map((block) => (block.type === 'text' ? block.text : ''))
    .join('')
  expect(transcriptText).toContain('DEMI_CODEX_TOOL_DONE')
})

async function runCodexRequest(options: {
  sessionId: string
  systemPrompt: string
  items: InferenceRequest['items']
  tools: InferenceRequest['tools']
  thinking: InferenceRequest['thinking']
}): Promise<ProviderEvent[]> {
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(), 90_000)
  const provider = new CodexProvider({ transport, maxRetries: 1 })
  const request: InferenceRequest = {
    sessionId: options.sessionId,
    turnId: `turn-${randomUUID()}`,
    requestId: `request-${randomUUID()}`,
    modelId,
    systemPrompt: options.systemPrompt,
    cwd: process.cwd(),
    items: options.items,
    tools: options.tools,
    thinking: options.thinking,
    cancel: controller.signal,
  }
  const events: ProviderEvent[] = []

  try {
    for await (const event of provider.run(request)) events.push(event)
  } finally {
    clearTimeout(timeout)
  }

  return events
}

async function expectCodexAuthAvailable(): Promise<void> {
  const state = await new FileCodexAuthStore({ codexHome: process.env.CODEX_HOME }).status()
  expect(state.status).toBe('authenticated')
}

function expectNoProviderErrors(events: ProviderEvent[]): void {
  expect(events.filter((event) => event.type === 'error')).toEqual([])
}

function textFrom(events: ProviderEvent[]): string {
  return events
    .filter((event): event is Extract<ProviderEvent, { type: 'text_delta' }> => event.type === 'text_delta')
    .map((event) => event.text)
    .join('')
}

function usageFrom(events: ProviderEvent[]): TokenUsage {
  const response = events.find((event): event is Extract<ProviderEvent, { type: 'response' }> => event.type === 'response')
  expect(response).toBeDefined()
  return response!.usage
}

function isVisibleThinkingEvent(event: ProviderEvent): boolean {
  return (
    (event.type === 'thinking_delta' && event.text.trim().length > 0)
    || (event.type === 'thinking_signature' && event.signature.length > 0)
    || (event.type === 'redacted_thinking' && event.data.length > 0)
  )
}

function parseTransport(value: string | undefined): 'auto' | 'sse' | 'websocket' {
  if (value === 'sse' || value === 'websocket' || value === 'auto') return value
  return 'sse'
}
