import process from 'node:process'
import { resolve } from 'node:path'
import type { CodexTransportMode } from '@demi/provider-codex'

export type ProviderId = 'claude-code' | 'codex'

export interface ServerOptions {
  port: number
  cwd: string
  provider: ProviderId
  modelId: string | null
  thinkingEffort: string | null
  serviceTierId: string | null
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
    thinkingEffort: null,
    serviceTierId: null,
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
    else if (arg === '--thinking') options.thinkingEffort = required(args, ++index, '--thinking')
    else if (arg === '--no-thinking') options.thinkingEffort = null
    else if (arg === '--service-tier') options.serviceTierId = required(args, ++index, '--service-tier')
    else if (arg === '--claude-path') options.claudePath = required(args, ++index, '--claude-path')
    else if (arg === '--codex-home') options.codexHome = required(args, ++index, '--codex-home')
    else if (arg === '--base-url') options.baseUrl = required(args, ++index, '--base-url')
    else if (arg === '--transport') options.transport = parseTransport(required(args, ++index, '--transport'))
    else if (arg === '--yield-after-ms') options.yieldAfterMs = Number(required(args, ++index, '--yield-after-ms'))
    else if (arg === '--timeout-ms') options.timeoutMs = Number(required(args, ++index, '--timeout-ms'))
    else throw new Error(`Unknown option: ${arg}`)
  }

  options.cwd = resolve(options.cwd)
  return options
}

export function providerConfigFor(provider: string, options: ServerOptions): Record<string, unknown> {
  if (provider === 'claude-code') {
    return {
      ...(options.claudePath ? { claudePath: options.claudePath } : {}),
    }
  }
  return {
    ...(options.codexHome ? { codexHome: options.codexHome } : {}),
    ...(options.baseUrl ? { baseUrl: options.baseUrl } : {}),
    transport: options.transport,
  }
}

function parseProvider(value: string): ProviderId {
  if (value === 'claude-code' || value === 'codex') return value
  throw new Error('--provider must be one of: claude-code, codex')
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
