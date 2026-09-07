import { expect, test } from 'bun:test'
import { createProviderQuota } from '@demicodes/provider'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createGrokBuildProvider } from '../provider'
import {
  createGrokBuildCredentials,
  openGrokCredentialPool,
  PoolAwareGrokAuthStore,
} from '../credentials'

function sampleEntry(email: string, key: string) {
  return {
    key,
    auth_mode: 'oidc',
    refresh_token: 'refresh',
    email,
    oidc_issuer: 'https://auth.x.ai',
    oidc_client_id: 'client',
    expires_at: '2099-01-01T00:00:00.000Z',
  }
}

test('grok credentials import entries and switch active', async () => {
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-grok-cred-'))
  const grokHome = await mkdtemp(join(tmpdir(), 'demi-grok-home-'))
  try {
    await writeFile(
      join(grokHome, 'auth.json'),
      JSON.stringify({
        'https://auth.x.ai::client-a': sampleEntry('a@x.ai', 'token-a'),
        'https://auth.x.ai::client-b': sampleEntry('b@x.ai', 'token-b'),
      }),
    )

    const pool = openGrokCredentialPool({ stateDir })
    const authStore = new PoolAwareGrokAuthStore(pool, { grokHome })
    const quota = createProviderQuota({
      providerId: 'grok-build',
      canProbe: true,
      probe: async () => ({ accountLabel: 'old-account', windows: [] }),
      observe: () => ({ accountLabel: 'old-account', windows: [] }),
    })
    const credentials = createGrokBuildCredentials(pool, authStore, { grokHome, quota })

    await credentials.importDefault!()
    const list = await credentials.list()
    expect(list.length).toBe(2)

    const a = list.find((e) => e.label === 'a@x.ai')
    const b = list.find((e) => e.label === 'b@x.ai')
    expect(a && b).toBeTruthy()

    await credentials.setActive(a!.id)
    expect((await authStore.resolveAuth()).email).toBe('a@x.ai')
    expect((await authStore.resolveAuth()).accessToken).toBe('token-a')

    await quota.probe()
    const oldObserver = quota.captureObserver()
    await credentials.setActive(b!.id)
    expect(quota.latest()).toBeNull()
    expect(oldObserver({})).toBeNull()
    expect(quota.latest()).toBeNull()
    expect((await authStore.resolveAuth()).email).toBe('b@x.ai')
    expect((await authStore.resolveAuth()).accessToken).toBe('token-b')
  } finally {
    await rm(stateDir, { recursive: true, force: true })
    await rm(grokHome, { recursive: true, force: true })
  }
})

test('createGrokBuildProvider exposes credentials by default', async () => {
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-grok-prov-'))
  try {
    const provider = createGrokBuildProvider({ stateDir })
    expect(provider.credentials).toBeDefined()
    expect(provider.credentials!.capability().mode).toBe('supported')
  } finally {
    await rm(stateDir, { recursive: true, force: true })
  }
})
