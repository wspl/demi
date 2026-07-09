import { isRecord, nonEmptyString } from '@demicodes/utils'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import type { InferenceRequest } from '@demicodes/provider'
import type { GrokResolvedAuth } from './auth'
import { defaultGrokHome } from './auth'

export const DEFAULT_GROK_BUILD_BASE_URL = 'https://cli-chat-proxy.grok.com/v1'
export const GROK_CLI_TOKEN_AUTH = 'xai-grok-cli'
export const GROK_CLIENT_SURFACE = 'grok-build'
/** Minimum cli-chat-proxy accepted version when no local Grok CLI version is available. */
export const DEFAULT_GROK_CLIENT_VERSION = '0.1.202'

export function buildGrokBuildHeaders(
  auth: GrokResolvedAuth,
  request?: Pick<InferenceRequest, 'sessionId' | 'requestId' | 'modelId'>,
  options?: {
    extra?: Record<string, string>
    clientVersion?: string
    grokHome?: string
  },
): Headers {
  const headers = new Headers(options?.extra)
  headers.set('authorization', `Bearer ${auth.accessToken}`)
  headers.set('X-XAI-Token-Auth', GROK_CLI_TOKEN_AUTH)
  headers.set('x-grok-client-surface', GROK_CLIENT_SURFACE)
  headers.set('x-grok-client-version', resolveGrokClientVersion(options?.clientVersion, options?.grokHome))
  if (request?.modelId) headers.set('x-grok-model-override', request.modelId)
  if (request?.sessionId) headers.set('x-grok-session-id', request.sessionId)
  if (request?.requestId) headers.set('x-grok-req-id', request.requestId)
  return headers
}

export function resolveGrokClientVersion(explicit?: string, grokHome?: string): string {
  const fromExplicit = nonEmptyString(explicit)
  if (fromExplicit) return fromExplicit
  const fromFile = readGrokCliVersion(grokHome ?? defaultGrokHome())
  return fromFile ?? DEFAULT_GROK_CLIENT_VERSION
}

function readGrokCliVersion(grokHome: string): string | null {
  try {
    const parsed = JSON.parse(readFileSync(join(grokHome, 'version.json'), 'utf8')) as unknown
    if (!isRecord(parsed)) return null
    return nonEmptyString(parsed.version) ?? null
  } catch {
    return null
  }
}
