import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { openBackend, login } from './session'

const json = (body: unknown, method = 'POST') => ({
  method,
  body: JSON.stringify(body),
  headers: { 'content-type': 'application/json' }
})

test(
  'Claude setup tokens stay private; accounts switch explicitly and the active account cannot be removed',
  async () => {
    const backend = await openBackend({
      dataDir: await mkdtemp(join(tmpdir(), 'demi-accounts-')),
      port: 0
    })
    try {
      expect(
        (await backend.session.fetch(
          '/api/providers/subscription-login',
          json({ providerType: 'claude-code' })
        )).status
      ).toBe(
        400
      )
      const created = await backend.session.fetch(
        '/api/providers/setup-token',
        json({ token: 'fixture-token-a', label: 'Claude' })
      )
      expect(created.status).toBe(201)
      const { provider } = await created.json() as { provider: { id: string } }
      const path = `/api/providers/${provider.id}/accounts`
      const added = await backend.session.fetch(
        path,
        json({ token: 'fixture-token-b' })
      )
      expect(added.status).toBe(201)
      const { account } = await added.json() as { account: { id: string } }
      const before = await (await backend.session.fetch(path)).json() as {
        accounts: Array<{ id: string }>;
        active: { credentialId: string }
      }
      expect(before.accounts).toHaveLength(2)
      expect(JSON.stringify(before)).not.toContain('fixture-token')
      expect(
        (await backend.session.fetch(
          `${path}/${before.active.credentialId}`,
          { method: 'DELETE' }
        )).status
      ).toBe(
        409
      )
      const switched = await backend.session.fetch(
        `${path}/active`,
        json({ credentialId: account.id }, 'PUT')
      )
      expect(await switched.json()).toMatchObject(
        { active: { credentialId: account.id } }
      )
      expect(
        (await backend.session.fetch(
          `${path}/${before.active.credentialId}`,
          { method: 'DELETE' }
        )).status
      ).toBe(
        204
      )
      expect(
        (await backend.session.fetch(
          `${path}/active`,
          json({ credentialId: 'missing' }, 'PUT')
        )).status
      ).toBe(
        404
      )
      await backend.session.fetch('/api/users', json({
        email: 'reader@example.test',
        password: 'reader-pass-1',
        role: 'user'
      }))
      const reader = await login(
        backend,
        'reader@example.test',
        'reader-pass-1'
      )
      expect((await reader.fetch(path, json({ token: 'not-allowed' }))).status).toBe(
        403
      )
      expect((await reader.fetch(path)).status).toBe(200)
      const catalog = await (await reader.fetch('/api/models')).json() as {
        providers: Array<{
          availability: {
            available: boolean;
            reason: string
          }
        }>
      }
      expect(catalog.providers[0]?.availability).toMatchObject(
        { available: false, reason: 'execution' }
      )
      expect(
        JSON.stringify(
          await (await reader.fetch(`/api/providers/${provider.id}/status`)).json()
        )
      ).not.toContain(
        'fixture-token'
      )
    } finally {
      await backend.close()
    }
  }
)

test(
  'adding a device-login account reserves the provider and cancellation releases it',
  async () => {
    const { mkdir, writeFile } = await import('node:fs/promises')
    const { defineProvider } = await import('@demicodes/provider')
    const { StubProvider } = await import('@demicodes/provider/testing')
    let attempts = 0
    let cancelled = 0
    const backend = await openBackend({
      dataDir: await mkdtemp(join(tmpdir(), 'demi-device-login-')),
      port: 0,
      providerTypes: {
        device: {
          credential: 'subscription',
          create: ({ providerId, label, vaultDir }) => defineProvider({
            id: providerId,
            displayName: label,
            credentials: {
              capability: () => ({ mode: 'supported', canBeginLogin: true }),
              list: () => [{ id: 'account', label: 'Fixture' }],
              getActive: () => ({
                credentialId: 'account',
                status: { status: 'authenticated' }
              }),
              setActive: () => ({
                credentialId: 'account',
                status: { status: 'authenticated' }
              }),
              beginLogin: async options => {
                attempts += 1
                if (attempts === 1) {
                  await mkdir(vaultDir, { recursive: true })
                  await writeFile(join(vaultDir, 'fixture'), 'fixture')
                  return { status: 'completed', credentialId: 'account' }
                }
                options!.onPending?.({
                  verificationUrl: 'https://example.test/device',
                  userCode: 'ABCD'
                })
                await new Promise<void>(resolve => {
                  const abort = () => {
                    cancelled += 1
                    resolve()
                  }
                  if (options!.signal!.aborted)
                    abort()
                  else options!.signal!.addEventListener(
                    'abort',
                    abort,
                    { once: true }
                  )
                })
                return { status: 'cancelled' }
              },
            },
            createRuntime: () => new StubProvider([]),
          })
        },
      }
    })
    try {
      const { login: started } = await (await backend.session.fetch(
        '/api/providers/subscription-login',
        json({ providerType: 'device' })
      )).json() as { login: { id: string } }
      let id = ''
      for (let i = 0; i < 100; i += 1) {
        const state = await (await backend.session.fetch(
          `/api/providers/subscription-login/${started.id}`
        )).json() as { login: { providerId?: string } }
        if (state.login.providerId) {
          id = state.login.providerId
          break
        }
        await Bun.sleep(5)
      }
      expect(id).not.toBe('')
      const path = `/api/providers/${id}`
      const added = await backend.session.fetch(
        `${path}/accounts/login`,
        { method: 'POST' }
      )
      expect(added.status).toBe(202)
      const { login: pending } = await added.json() as { login: { id: string } }
      expect((await backend.session.fetch(path, { method: 'DELETE' })).status).toBe(
        409
      )
      expect(
        (await backend.session.fetch(path, json({ label: 'busy' }, 'PATCH'))).status
      ).toBe(
        409
      )
      expect(
        (await backend.session.fetch(
          `/api/providers/subscription-login/${pending.id}`,
          { method: 'DELETE' }
        )).status
      ).toBe(
        204
      )
      expect(cancelled).toBe(1)
      expect(
        (await backend.session.fetch(path, json({ label: 'ready' }, 'PATCH'))).status
      ).toBe(
        200
      )
    } finally {
      await backend.close()
    }
  }
)
