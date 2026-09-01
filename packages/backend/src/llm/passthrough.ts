import {
  PoolAwareClaudeCodeAuthStore,
  openClaudeCodeCredentialPool,
  type ClaudeCodeAuthStore,
} from '@demicodes/provider-claude-code'
import { errorMessage } from '@demicodes/utils'
import type { ProviderAssembly } from './assembly'

const DEFAULT_ANTHROPIC_UPSTREAM = 'https://api.anthropic.com'

/** Request headers never forwarded upstream (auth is replaced, the rest are hop-by-hop). */
const DROPPED_HEADERS = new Set(['authorization', 'x-api-key', 'host', 'content-length', 'connection'])

/**
 * The Anthropic passthrough: the one endpoint Claude Code CLI processes talk
 * to (`ANTHROPIC_BASE_URL`). It authenticates the backend-issued passthrough
 * token, swaps in the connection's vault OAuth token, and forwards exactly
 * one request class — `POST /v1/messages*`. Runners hold zero credentials;
 * every CLI request is credentialed and meterable here.
 */
export class AnthropicPassthrough {
  private readonly tokens = new Map<string, { connectionId: string }>()
  private readonly stores = new Map<string, ClaudeCodeAuthStore>()
  private readonly upstream: string
  private readonly fetchImpl: typeof fetch

  constructor(
    private readonly assembly: ProviderAssembly,
    options: { upstreamBaseUrl?: string; fetch?: typeof fetch } = {},
  ) {
    this.upstream = (options.upstreamBaseUrl ?? DEFAULT_ANTHROPIC_UPSTREAM).replace(/\/$/, '')
    this.fetchImpl = options.fetch ?? fetch
  }

  /** Issues a session-scoped passthrough token for one connection (`CLAUDE_CODE_OAUTH_TOKEN`). */
  mintToken(connectionId: string): string {
    const token = Buffer.from(crypto.getRandomValues(new Uint8Array(32))).toString('hex')
    this.tokens.set(token, { connectionId })
    return token
  }

  revokeToken(token: string): void {
    this.tokens.delete(token)
  }

  async handle(request: Request, subpath: string): Promise<Response> {
    if (request.method !== 'POST' || !(subpath === '/v1/messages' || subpath.startsWith('/v1/messages/'))) {
      return json(404, { code: 'unsupported_request', message: 'The passthrough forwards POST /v1/messages only' })
    }
    const bearer = request.headers.get('authorization')?.replace(/^Bearer\s+/i, '')
    const grant = bearer ? this.tokens.get(bearer) : undefined
    if (!grant) return json(401, { code: 'unauthorized', message: 'Unknown passthrough token' })

    let accessToken: string
    try {
      accessToken = (await this.authStore(grant.connectionId).resolveAccess()).accessToken
    } catch (error) {
      return json(502, { code: 'auth_unavailable', message: errorMessage(error) })
    }

    const url = new URL(request.url)
    const headers = new Headers()
    request.headers.forEach((value, name) => {
      if (!DROPPED_HEADERS.has(name.toLowerCase())) headers.set(name, value)
    })
    headers.set('authorization', `Bearer ${accessToken}`)
    const upstream = await this.fetchImpl(`${this.upstream}${subpath}${url.search}`, {
      method: 'POST',
      headers,
      body: request.body,
      ...( { duplex: 'half' } as Record<string, unknown>),
    })
    return new Response(upstream.body, { status: upstream.status, headers: upstream.headers })
  }

  private authStore(connectionId: string): ClaudeCodeAuthStore {
    let store = this.stores.get(connectionId)
    if (!store) {
      store = new PoolAwareClaudeCodeAuthStore(
        openClaudeCodeCredentialPool({ stateDir: this.assembly.vaultDir(connectionId) }),
      )
      this.stores.set(connectionId, store)
    }
    return store
  }
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
}
