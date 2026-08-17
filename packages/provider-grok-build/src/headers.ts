import { isRecord, nonEmptyString } from '@demicodes/utils'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import type { InferenceRequest } from '@demicodes/provider'
import type { GrokResolvedAuth } from './auth'
import { defaultGrokHome } from './auth'

export const DEFAULT_GROK_BUILD_BASE_URL = 'https://cli-chat-proxy.grok.com/v1'
export const GROK_CLI_TOKEN_AUTH = 'xai-grok-cli'
const GROK_AUTHENTICATE_RESPONSE = 'authenticate-response'
const GROK_CLIENT_IDENTIFIER = 'grok-shell'
const GROK_CLIENT_MODE = 'interactive'
/** Official Grok CLI crate version, used when no local `~/.grok/version.json` exists. */
const DEFAULT_GROK_CLIENT_VERSION = '1.0.5'

export function buildGrokBuildHeaders(
  auth: GrokResolvedAuth,
  request?: Pick<InferenceRequest, 'sessionId' | 'requestId' | 'modelId' | 'turnId'>,
  options?: {
    extra?: Record<string, string>
    clientVersion?: string
    grokHome?: string
  },
): Headers {
  const headers = new Headers(options?.extra)
  headers.set('authorization', `Bearer ${auth.accessToken}`)
  headers.set('X-XAI-Token-Auth', GROK_CLI_TOKEN_AUTH)
  headers.set('x-authenticateresponse', GROK_AUTHENTICATE_RESPONSE)
  headers.set('x-grok-client-version', resolveGrokClientVersion(options?.clientVersion, options?.grokHome))
  headers.set('x-grok-client-identifier', nonEmptyString(process.env.GROK_CLIENT_NAME) ?? GROK_CLIENT_IDENTIFIER)
  headers.set('x-grok-client-mode', GROK_CLIENT_MODE)
  if (auth.userId) {
    headers.set('x-userid', auth.userId)
    headers.set('x-grok-user-id', auth.userId)
  }
  if (auth.email) headers.set('x-email', auth.email)
  if (request?.modelId) headers.set('x-grok-model-override', request.modelId)
  if (request?.sessionId) {
    headers.set('x-grok-session-id', request.sessionId)
    headers.set('x-grok-conv-id', request.sessionId)
  }
  if (request?.requestId) headers.set('x-grok-req-id', request.requestId)
  if (request?.turnId) headers.set('x-grok-turn-idx', request.turnId)
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
