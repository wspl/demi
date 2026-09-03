import { existsSync } from 'node:fs'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demicodes/core'
import { AgentClient, createWebSocketClientTransport } from '@demicodes/agent'
import { FileCredentialPool } from '@demicodes/provider/credentials-pool'
import { startTinyjsRunner } from '@demicodes/runner/testing'
import { delay, waitFor } from '@demicodes/utils'
import { LocalControlService } from '../storage/control'
import { openSqliteDatabase } from '../storage/database'
import { ConnectionVault } from '../vault/connections'
import { loadOrCreateInstanceSecret } from '../vault/secret'
import { createClaudeCodeProvider } from '@demicodes/provider-claude-code'
import { openBackend } from './session'

// M5 step 2 acceptance, tier 2: the full Claude Code chain — backend spawns
// the real `claude` CLI on the session's claimed runner; the provider
// resolves the vault OAuth token and injects it into the CLI env; the CLI
// talks its native Anthropic wire directly to a mock upstream (pointed there
// by the provider's public env overlay — a test tool). No real model is ever
// called. Skipped when no `claude` binary is installed.

const claudeBinary = Bun.which('claude')
const chain = claudeBinary ? test : test.skip

function sse(events: Array<[string, unknown]>): Response {
  const body = events.map(([event, data]) => `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`).join('')
  return new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } })
}

chain('claude-code on a runner: vault token in the CLI env, native wire to a mock upstream', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-claude-chain-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-claude-chain-state-'))
  const runnerDir = await mkdtemp(join(tmpdir(), 'demi-claude-chain-runner-'))

  const upstreamAuths: Array<string | null> = []
  const upstream = Bun.serve({
    port: 0,
    fetch: async (request) => {
      const path = new URL(request.url).pathname
      if (request.method === 'POST' && path === '/v1/messages') {
        upstreamAuths.push(request.headers.get('authorization'))
        return sse([
          ['message_start', {
            type: 'message_start',
            message: {
              id: 'msg_mock',
              type: 'message',
              role: 'assistant',
              model: 'claude-mock',
              content: [],
              stop_reason: null,
              stop_sequence: null,
              usage: { input_tokens: 25, output_tokens: 1 },
            },
          }],
          ['content_block_start', { type: 'content_block_start', index: 0, content_block: { type: 'text', text: '' } }],
          ['content_block_delta', { type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'OK from mock upstream' } }],
          ['content_block_stop', { type: 'content_block_stop', index: 0 }],
          ['message_delta', { type: 'message_delta', delta: { stop_reason: 'end_turn', stop_sequence: null }, usage: { output_tokens: 6 } }],
          ['message_stop', { type: 'message_stop' }],
        ])
      }
      return Response.json({ error: { type: 'not_found_error', message: `mock upstream: no ${request.method} ${path}` } }, { status: 404 })
    },
  })

  const backend = await openBackend({
    dataDir,
    port: 0,
    runner: { pingIntervalMs: 0 },
    providerTypes: {
      // The builtin factory, plus the provider's public env overlay pointing
      // the CLI at the mock upstream.
      'claude-code': ({ connectionId, label, vaultDir, session }) =>
        createClaudeCodeProvider({
          id: connectionId,
          displayName: label,
          stateDir: vaultDir,
          ...(session ? { spawn: session.spawn } : {}),
          env: { ANTHROPIC_BASE_URL: `http://localhost:${upstream.port}` },
        }),
    },
  })

  // Pair the runner.
  const runner = await startTinyjsRunner({ backendUrl: backend.url, stateDir, home: runnerDir, name: 'chain-device' })
  await waitFor(() => runner.codes.length > 0, () => runner.log.join('\n'), { timeoutMs: 10_000 })
  const codes = runner.codes
  const claimResponse = await backend.session.fetch(`/api/devices/claim`, {
    method: 'POST',
    body: JSON.stringify({ code: codes[0] }),
    headers: { 'content-type': 'application/json' },
  })
  const { device } = (await claimResponse.json()) as { device: { id: string } }

  // A claude-code subscription connection whose vault pool holds the (fake)
  // OAuth secret — written control-plane-side exactly as a completed login
  // would leave it. The runner never sees this value.
  const controlDb = openSqliteDatabase(join(dataDir, 'control.sqlite'))
  const control = new LocalControlService(controlDb)
  const vault = new ConnectionVault(control, loadOrCreateInstanceSecret(dataDir))
  const connection = await vault.create({
    ownerUserId: backend.session.user.id,
    label: 'Claude subscription',
    config: { kind: 'subscription', provider: 'claude-code' },
  })
  const pool = new FileCredentialPool({
    stateDir: join(dataDir, 'vault', connection.id),
    providerKey: 'claude-code',
    secretFileName: 'oauth.json',
  })
  const meta = await pool.writeEntry(
    { id: 'cred-chain', label: 'chain@example.com', updatedAt: new Date().toISOString() },
    JSON.stringify({ accessToken: 'vault-oauth-token', expiresAt: new Date(Date.now() + 3_600_000).toISOString() }),
  )
  await pool.setActiveId(meta.id)

  // Conversation bound to the runner workspace.
  const created = await backend.session.fetch(`/api/conversations`, { method: 'POST' })
  const { conversation } = (await created.json()) as { conversation: { id: string } }
  const workspace = await control.createWorkspace({
    userId: backend.session.user.id,
    deviceId: device.id,
    path: runnerDir,
    name: 'chain workspace',
  })
  await control.setConversationWorkspace(conversation.id, workspace.id)
  controlDb.close()

  const socket = backend.session.socket(`/api/conversations/${conversation.id}/stream`)
  await new Promise<void>((resolve) => socket.addEventListener('open', () => resolve(), { once: true }))
  const client = new AgentClient(createWebSocketClientTransport(socket as never))
  const model: ModelSelection = {
    providerId: connection.id,
    model: {
      id: 'claude-sonnet-4-6',
      name: 'Claude Sonnet',
      contextWindow: 200_000,
      inputLimit: null,
      thinking: [],
      acceptedExtensions: [],
    },
    thinking: null,
  }
  await client.open({ providerId: connection.id, model }, '/ignored', 'ignored')
  await client.send([{ type: 'text', text: 'Reply with exactly OK.' }])

  // The mock's answer streamed back through CLI → runner → backend → browser.
  const texts = client
    .transcript()
    .blocks.filter((block) => block.type === 'text')
    .map((block) => (block.type === 'text' ? block.text : ''))
  expect(texts.join(' ')).toContain('OK from mock upstream')

  // Token swap happened at the passthrough: the upstream saw the vault token,
  // and the CLI-side env token is a different, backend-minted value.
  expect(upstreamAuths.length).toBeGreaterThan(0)
  for (const auth of upstreamAuths) expect(auth).toBe('Bearer vault-oauth-token')

  // Device-config isolation: the CLI's config home lived inside the
  // workspace artifacts dir, not the device's ~/.claude.
  expect(existsSync(join(runnerDir, '.demi-artifacts', 'claude-config'))).toBe(true)

  // Metering: the CLI turn's usage landed in the ledger.
  await delay(100)
  const usage = (await (await backend.session.fetch(`/api/usage`)).json()) as {
    totals: Array<{ connectionId: string; requests: number }>
  }
  expect(usage.totals.some((row) => row.connectionId === connection.id && row.requests >= 1)).toBe(true)

  await client.close()
  await runner.stop()
  await backend.close()
  upstream.stop(true)
}, 120_000)
