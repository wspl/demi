import type { InferenceRequest } from '@demicodes/provider'
import type { GrokResolvedAuth } from './auth'

export const DEFAULT_GROK_BUILD_BASE_URL = 'https://cli-chat-proxy.grok.com/v1'
export const GROK_CLI_TOKEN_AUTH = 'xai-grok-cli'
export const GROK_CLIENT_SURFACE = 'grok-build'

export function buildGrokBuildHeaders(
  auth: GrokResolvedAuth,
  request?: Pick<InferenceRequest, 'sessionId' | 'requestId' | 'modelId'>,
  extra?: Record<string, string>,
): Headers {
  const headers = new Headers(extra)
  headers.set('authorization', `Bearer ${auth.accessToken}`)
  headers.set('X-XAI-Token-Auth', GROK_CLI_TOKEN_AUTH)
  headers.set('x-grok-client-surface', GROK_CLIENT_SURFACE)
  if (request?.modelId) headers.set('x-grok-model-override', request.modelId)
  if (request?.sessionId) headers.set('x-grok-session-id', request.sessionId)
  if (request?.requestId) headers.set('x-grok-req-id', request.requestId)
  return headers
}
