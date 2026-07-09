import { expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createClaudeCodeProvider } from '../provider'
import {
  createClaudeCodeCredentials,
  openClaudeCodeCredentialPool,
  PoolAwareClaudeCodeAuthStore,
} from '../credentials'
import { StaticClaudeCodeAuthStore } from '../auth'
import { buildClaudeEnv } from '../cli'

test('claude credentials add/setActive and pool-aware resolve', async () => {
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-claude-cred-'))
  try {
    const pool = openClaudeCodeCredentialPool({ stateDir })
    const authStore = new PoolAwareClaudeCodeAuthStore(pool)
    const credentials = createClaudeCodeCredentials(pool, authStore)

    const a = await credentials.add!({ accessToken: 'token-a', subscriptionType: 'pro' })
    const b = await credentials.add!({ accessToken: 'token-b', subscriptionType: 'max' })
    expect((await credentials.list()).length).toBe(2)

    await credentials.setActive(a.id)
    expect((await authStore.resolveAccess()).accessToken).toBe('token-a')
    await credentials.setActive(b.id)
    expect((await authStore.resolveAccess()).accessToken).toBe('token-b')
    expect((await credentials.getActive()).status).toMatchObject({
      status: 'authenticated',
      accountLabel: 'max',
    })
  } finally {
    await rm(stateDir, { recursive: true, force: true })
  }
})

test('buildClaudeEnv overlays CLAUDE_CODE_OAUTH_TOKEN', () => {
  const env = buildClaudeEnv({ PATH: '/bin' }, { oauthAccessToken: 'sk-test' })
  expect(env.CLAUDE_CODE_OAUTH_TOKEN).toBe('sk-test')
  expect(env.DISABLE_AUTO_COMPACT).toBe('1')
})

test('createClaudeCodeProvider wires credentials and auth status', async () => {
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-claude-prov-'))
  try {
    const provider = createClaudeCodeProvider({ stateDir })
    expect(provider.credentials).toBeDefined()
    await provider.credentials!.add!({ accessToken: 'tok', subscriptionType: 'pro' })
    const status = await provider.auth!.status()
    expect(status).toMatchObject({ status: 'authenticated', accountLabel: 'pro' })
  } finally {
    await rm(stateDir, { recursive: true, force: true })
  }
})

test('custom authStore skips credentials surface', () => {
  const provider = createClaudeCodeProvider({
    authStore: new StaticClaudeCodeAuthStore({ accessToken: 'x' }),
  })
  expect(provider.credentials).toBeUndefined()
})
