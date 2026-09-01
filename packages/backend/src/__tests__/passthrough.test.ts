import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { FileCredentialPool } from '@demicodes/provider/credentials-pool'
import { builtinProviderTypes, ProviderAssembly } from '../llm/assembly'
import { AnthropicPassthrough } from '../llm/passthrough'
import { LocalControlService } from '../storage/control'
import { openSqliteDatabase } from '../storage/database'
import { CONTROL_MIGRATIONS, migrate } from '../storage/migrations'
import { ConnectionVault } from '../vault/connections'
import { createBackend } from '../index'

// M5 step 2: the passthrough swaps the backend-issued token for the vault
// OAuth token and forwards exactly one request class. Runners and CLI
// processes never see the real credential.

async function seededPassthrough(upstreamBaseUrl: string) {
  const vaultRoot = await mkdtemp(join(tmpdir(), 'demi-passthrough-vault-'))
  const db = openSqliteDatabase(':memory:')
  migrate(db, CONTROL_MIGRATIONS)
  const control = new LocalControlService(db)
  await control.ensureUser({ id: 'u1', username: 'local', role: 'master' })
  const vault = new ConnectionVault(control, crypto.getRandomValues(new Uint8Array(32)))
  const assembly = new ProviderAssembly(vault, builtinProviderTypes(), vaultRoot)
  const connection = await vault.create({
    ownerUserId: 'u1',
    label: 'Claude sub',
    config: { kind: 'subscription', provider: 'claude-code' },
  })

  // The connection's vault pool holds the real OAuth secret.
  const pool = new FileCredentialPool({
    stateDir: assembly.vaultDir(connection.id),
    providerKey: 'claude-code',
    secretFileName: 'oauth.json',
  })
  const meta = await pool.writeEntry(
    { id: 'cred-test', label: 'tester@example.com', updatedAt: new Date().toISOString() },
    JSON.stringify({ accessToken: 'real-vault-token', expiresAt: new Date(Date.now() + 3_600_000).toISOString() }),
  )
  await pool.setActiveId(meta.id)

  const passthrough = new AnthropicPassthrough(assembly, { upstreamBaseUrl })
  return { passthrough, connectionId: connection.id, close: () => db.close() }
}

test('passthrough swaps the token, forwards POST /v1/messages only, and rejects unknown tokens', async () => {
  const seen: Array<{ path: string; auth: string | null; apiKey: string | null; version: string | null; body: unknown }> = []
  const upstream = Bun.serve({
    port: 0,
    fetch: async (request) => {
      seen.push({
        path: new URL(request.url).pathname,
        auth: request.headers.get('authorization'),
        apiKey: request.headers.get('x-api-key'),
        version: request.headers.get('anthropic-version'),
        body: await request.json(),
      })
      return Response.json({ id: 'msg_1', content: [] })
    },
  })
  const { passthrough, connectionId, close } = await seededPassthrough(`http://localhost:${upstream.port}`)
  const token = passthrough.mintToken(connectionId)

  const request = (path: string, init: RequestInit & { token?: string } = {}) =>
    passthrough.handle(
      new Request(`http://backend.local/api/passthrough/anthropic${path}`, {
        method: init.method ?? 'POST',
        headers: {
          authorization: `Bearer ${init.token ?? token}`,
          'anthropic-version': '2023-06-01',
          'content-type': 'application/json',
        },
        ...(init.method === 'GET' ? {} : { body: JSON.stringify({ model: 'claude-x', messages: [] }) }),
      }),
      path,
    )

  // The one allowed class round-trips with the swapped credential.
  const ok = await request('/v1/messages')
  expect(ok.status).toBe(200)
  expect(await ok.json()).toEqual({ id: 'msg_1', content: [] })
  expect(seen).toHaveLength(1)
  expect(seen[0]?.auth).toBe('Bearer real-vault-token')
  expect(seen[0]?.apiKey).toBeNull()
  expect(seen[0]?.version).toBe('2023-06-01')
  expect(seen[0]?.body).toEqual({ model: 'claude-x', messages: [] })

  // Single request class: anything else never reaches the upstream.
  expect((await request('/v1/complete')).status).toBe(404)
  expect((await request('/v1/messages', { method: 'GET' })).status).toBe(404)
  expect((await request('/v1/messagesabc')).status).toBe(404)
  // Unknown or revoked tokens are refused.
  expect((await request('/v1/messages', { token: 'wrong' })).status).toBe(401)
  passthrough.revokeToken(token)
  expect((await request('/v1/messages')).status).toBe(401)
  expect(seen).toHaveLength(1)

  upstream.stop(true)
  close()
})

test('the passthrough route is mounted and refuses without touching any upstream', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-passthrough-http-'))
  const backend = await createBackend({ dataDir, port: 0 })
  const bad = await fetch(`${backend.url}/api/passthrough/anthropic/v1/messages`, {
    method: 'POST',
    headers: { authorization: 'Bearer nope' },
    body: '{}',
  })
  expect(bad.status).toBe(401)
  const wrongClass = await fetch(`${backend.url}/api/passthrough/anthropic/v1/models`, { method: 'POST', body: '{}' })
  expect(wrongClass.status).toBe(404)
  expect((await wrongClass.json() as { code: string }).code).toBe('unsupported_request')
  await backend.close()
})
