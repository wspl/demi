import process from 'node:process'
import { resolve } from 'node:path'
import type { ThinkingEffort } from '@demicodes/core'
import type { CodexTransportMode } from '@demicodes/provider-codex'
import type { OpenAIApiWireApi } from '@demicodes/provider-openai-api'
import { helpText } from './input-loop'
import { writeLine } from './output'

export interface ReplOptions {
  provider: 'claude-code' | 'codex' | 'openai' | 'anthropic'
  cwd: string
  modelId: string | null
  thinkingEffort: ThinkingEffort | null
  serviceTierId: string | null
  openAIWireApi: OpenAIApiWireApi
  claudePath?: string
  codexHome?: string
  baseUrl?: string
  transport: CodexTransportMode
}

export function parseArgs(args: string[]): ReplOptions {
  let provider: ReplOptions['provider'] = parseProvider(process.env.DEMI_PROVIDER ?? 'claude-code')
  let cwd = process.cwd()
  let modelId: string | null = null
  let thinkingEffort = parseThinkingEffort(envThinkingValue(provider), envThinkingSource(provider))
  let thinkingProvided = false
  let serviceTierId: string | null = null
  let claudePath: string | undefined
  let codexHome: string | undefined = process.env.CODEX_HOME
  let baseUrl: string | undefined
  let openAIWireApi = parseOpenAIWireApi(process.env.DEMI_OPENAI_WIRE_API ?? 'responses')
  let transport: CodexTransportMode = 'auto'

  for (let index = 0; index < args.length; index++) {
    const arg = args[index]
    if (arg === '--help' || arg === '-h') {
      printUsage()
      process.exit(0)
    }
    if (arg === '--cwd') cwd = requiredValue(args, ++index, '--cwd')
    else if (arg === '--provider') provider = parseProvider(requiredValue(args, ++index, '--provider'))
    else if (arg === '--model') {
      modelId = requiredValue(args, ++index, '--model')
    }
    else if (arg === '--thinking') {
      thinkingEffort = parseThinkingEffort(requiredValue(args, ++index, '--thinking'), '--thinking')
      thinkingProvided = true
    } else if (arg === '--no-thinking') {
      thinkingEffort = null
      thinkingProvided = true
    }
    else if (arg === '--service-tier') serviceTierId = requiredValue(args, ++index, '--service-tier')
    else if (arg === '--claude-path') claudePath = requiredValue(args, ++index, '--claude-path')
    else if (arg === '--codex-home') codexHome = requiredValue(args, ++index, '--codex-home')
    else if (arg === '--base-url') baseUrl = requiredValue(args, ++index, '--base-url')
    else if (arg === '--openai-wire-api') openAIWireApi = parseOpenAIWireApi(requiredValue(args, ++index, '--openai-wire-api'))
    else if (arg === '--transport') transport = parseCodexTransport(requiredValue(args, ++index, '--transport'))
    else if (!arg.startsWith('-')) cwd = arg
    else throw new Error(`Unknown option: ${arg}`)
  }

  if (!thinkingProvided) thinkingEffort = parseThinkingEffort(envThinkingValue(provider), envThinkingSource(provider))

  return {
    provider,
    cwd: resolve(cwd),
    modelId,
    thinkingEffort,
    serviceTierId,
    openAIWireApi,
    claudePath,
    codexHome,
    baseUrl,
    transport,
  }
}

function envThinkingValue(provider: ReplOptions['provider']): string | null {
  if (provider === 'codex') return process.env.DEMI_CODEX_THINKING ?? process.env.DEMI_CLAUDE_CODE_THINKING ?? null
  if (provider === 'openai') return process.env.DEMI_OPENAI_THINKING ?? null
  if (provider === 'anthropic') return process.env.DEMI_ANTHROPIC_THINKING ?? null
  return process.env.DEMI_CLAUDE_CODE_THINKING ?? null
}

function envThinkingSource(provider: ReplOptions['provider']): string {
  if (provider === 'codex' && process.env.DEMI_CODEX_THINKING !== undefined) return 'DEMI_CODEX_THINKING'
  if (provider === 'openai') return 'DEMI_OPENAI_THINKING'
  if (provider === 'anthropic') return 'DEMI_ANTHROPIC_THINKING'
  return 'DEMI_CLAUDE_CODE_THINKING'
}

function parseProvider(value: string): ReplOptions['provider'] {
  if (value === 'claude-code' || value === 'codex' || value === 'openai' || value === 'anthropic') return value
  throw new Error('--provider must be one of: claude-code, codex, openai, anthropic')
}

function parseOpenAIWireApi(value: string): OpenAIApiWireApi {
  if (value === 'responses' || value === 'chat-completions') return value
  throw new Error('--openai-wire-api must be one of: responses, chat-completions')
}

function parseCodexTransport(value: string): CodexTransportMode {
  if (value === 'auto' || value === 'sse' || value === 'websocket') return value
  throw new Error('--transport must be one of: auto, sse, websocket')
}

function requiredValue(args: string[], index: number, flag: string): string {
  const value = args[index]
  if (!value) throw new Error(`${flag} requires a value`)
  return value
}

function parseThinkingEffort(value: string | null, source: string): ThinkingEffort | null {
  if (value === null || value === '') return null
  const effort = value.trim()
  if (!effort) return null
  if (effort.startsWith('-')) throw new Error(`${source} must be a provider-supported thinking effort id`)
  return effort
}

export function printUsage(): void {
  writeLine(`Usage: bun run repl -- [cwd] [options]

Options:
  --cwd <path>             Working directory. Defaults to current directory.
  --provider <id>          Provider: claude-code, codex, openai, anthropic. Defaults to claude-code.
  --model <id>             Full model id. Defaults to the provider model catalog selection.
  --thinking <effort>      Provider-supported thinking effort id.
  --no-thinking            Do not request an explicit thinking effort. This is the default.
  --service-tier <id>      Provider-supported service tier id.
  --claude-path <path>     Path to claude CLI. Defaults to claude on PATH.
  --codex-home <path>      Codex home containing auth.json. Defaults to CODEX_HOME or ~/.codex.
  --base-url <url>         Override the selected HTTP provider base URL.
  --openai-wire-api <api>  OpenAI wire API: responses, chat-completions. Defaults to responses.
  --transport <mode>       Codex transport: auto, sse, websocket. Defaults to auto.

${helpText}`)
}
