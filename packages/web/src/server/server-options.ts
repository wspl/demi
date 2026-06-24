import process from 'node:process'
import { resolve } from 'node:path'
import type { CodexTransportMode } from '@demi/provider-codex'
import type { OpenAIApiWireApi } from '@demi/provider-openai-api'

export type ProviderId = 'claude-code' | 'codex' | 'openai' | 'anthropic'

export interface ServerOptions {
  port: number
  cwd: string
  provider: ProviderId
  modelId: string | null
  modelDisplayName: string | null
  thinkingEffort: string | null
  serviceTierId: string | null
  openAIWireApi: OpenAIApiWireApi
  claudePath?: string
  codexHome?: string
  baseUrl?: string
  transport: CodexTransportMode
  yieldAfterMs: number
  timeoutMs: number
}

export function parseServerOptions(args: string[]): ServerOptions {
  const options: ServerOptions = {
    port: Number(process.env.DEMI_WEB_PORT ?? '4280'),
    cwd: process.cwd(),
    provider: parseProvider(process.env.DEMI_PROVIDER ?? 'claude-code'),
    modelId: null,
    modelDisplayName: process.env.DEMI_MODEL_DISPLAY_NAME ?? null,
    thinkingEffort: null,
    serviceTierId: null,
    openAIWireApi: parseOpenAIWireApi(process.env.DEMI_OPENAI_WIRE_API ?? 'responses'),
    codexHome: process.env.CODEX_HOME,
    transport: 'auto',
    yieldAfterMs: 10_000,
    timeoutMs: 120_000,
  }

  for (let index = 0; index < args.length; index++) {
    const arg = args[index]
    if (arg === '--port') options.port = Number(required(args, ++index, '--port'))
    else if (arg === '--cwd') options.cwd = required(args, ++index, '--cwd')
    else if (arg === '--provider') options.provider = parseProvider(required(args, ++index, '--provider'))
    else if (arg === '--model') options.modelId = required(args, ++index, '--model')
    else if (arg === '--model-display-name') options.modelDisplayName = required(args, ++index, '--model-display-name')
    else if (arg === '--thinking') options.thinkingEffort = required(args, ++index, '--thinking')
    else if (arg === '--no-thinking') options.thinkingEffort = null
    else if (arg === '--service-tier') options.serviceTierId = required(args, ++index, '--service-tier')
    else if (arg === '--openai-wire-api') options.openAIWireApi = parseOpenAIWireApi(required(args, ++index, '--openai-wire-api'))
    else if (arg === '--claude-path') options.claudePath = required(args, ++index, '--claude-path')
    else if (arg === '--codex-home') options.codexHome = required(args, ++index, '--codex-home')
    else if (arg === '--base-url') options.baseUrl = required(args, ++index, '--base-url')
    else if (arg === '--transport') options.transport = parseTransport(required(args, ++index, '--transport'))
    else if (arg === '--yield-after-ms') options.yieldAfterMs = Number(required(args, ++index, '--yield-after-ms'))
    else if (arg === '--timeout-ms') options.timeoutMs = Number(required(args, ++index, '--timeout-ms'))
    else throw new Error(`Unknown option: ${arg}`)
  }

  if (options.modelDisplayName && !options.modelId) throw new Error('--model-display-name requires --model')
  options.cwd = resolve(options.cwd)
  return options
}

function parseProvider(value: string): ProviderId {
  if (value === 'claude-code' || value === 'codex' || value === 'openai' || value === 'anthropic') return value
  throw new Error('--provider must be one of: claude-code, codex, openai, anthropic')
}

function parseOpenAIWireApi(value: string): OpenAIApiWireApi {
  if (value === 'responses' || value === 'chat-completions') return value
  throw new Error('--openai-wire-api must be one of: responses, chat-completions')
}

function parseTransport(value: string): CodexTransportMode {
  if (value === 'auto' || value === 'sse' || value === 'websocket') return value
  throw new Error('--transport must be one of: auto, sse, websocket')
}

function required(args: string[], index: number, flag: string): string {
  const value = args[index]
  if (!value) throw new Error(`${flag} requires a value`)
  return value
}
