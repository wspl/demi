import { expect, test } from 'bun:test'
import { Buffer } from 'node:buffer'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createCodexProvider } from '../provider'
import { openCodexCredentialPool, PoolAwareCodexAuthStore } from '../credentials'
import { createCodexCredentials } from '../credentials'

function jwt(payload: Record<string, unknown>): string {
  const header = Buffer.from(JSON.stringify({ alg: 'none', typ: 'JWT' })).toString('base64url')
  const body = Buffer.from(JSON.stringify(payload)).toString('base64url')
  return `${header}.${body}.sig`
}

async function writeVendorAuth(codexHome: string, email: string, accountId: string) {
  const access = jwt({
    exp: 1_900_000_000,
    email,
    'https://api.openai.com/auth': { chatgpt_account_id: accountId },
  })
  const id = jwt({
    email,
    'https://api.openai.com/auth': { chatgpt_account_id: accountId },
  })
  await writeFile(
    join(codexHome, 'auth.json'),
    JSON.stringify({
      auth_mode: 'chatgpt',
      tokens: {
        id_token: id,
        access_token: access,
        refresh_token: 'refresh',
        account_id: accountId,
      },
      last_refresh: new Date().toISOString(),
    }),
  )
}

test('codex credentials importDefault, setActive, and resolve via pool', async () => {
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-cred-state-'))
  const codexA = await mkdtemp(join(tmpdir(), 'demi-codex-a-'))
  const codexB = await mkdtemp(join(tmpdir(), 'demi-codex-b-'))
  try {
    await writeVendorAuth(codexA, 'a@example.com', 'acct-a')
    await writeVendorAuth(codexB, 'b@example.com', 'acct-b')

    const pool = openCodexCredentialPool({ stateDir })
    const authStore = new PoolAwareCodexAuthStore(pool, { codexHome: codexA })
    const credentials = createCodexCredentials(pool, authStore, { codexHome: codexA })

    const importedA = await credentials.importDefault!()
    expect(importedA.label).toBe('a@example.com')
    expect((await credentials.list()).length).toBe(1)

    // Switch vendor home and import second account
    const credentialsB = createCodexCredentials(pool, authStore, { codexHome: codexB })
    const importedB = await credentialsB.importDefault!()
    expect(importedB.label).toBe('b@example.com')
    expect((await credentials.list()).length).toBe(2)

    await credentials.setActive(importedA.id)
    const activeA = await credentials.getActive()
    expect(activeA.credentialId).toBe(importedA.id)
    expect(activeA.status).toMatchObject({ status: 'authenticated', accountLabel: 'a@example.com' })

    await credentials.setActive(importedB.id)
    const activeB = await credentials.getActive()
    expect(activeB.credentialId).toBe(importedB.id)
    expect(activeB.status).toMatchObject({ status: 'authenticated', accountLabel: 'b@example.com' })

    const auth = await authStore.resolveAuth()
    expect(auth.kind === 'chatgpt' ? auth.email : null).toBe('b@example.com')

    // active pointer file exists
    const activeRaw = await readFile(join(stateDir, 'credentials', 'codex', 'active'), 'utf8')
    expect(activeRaw.trim()).toBe(importedB.id)
  } finally {
    await rm(stateDir, { recursive: true, force: true })
    await rm(codexA, { recursive: true, force: true })
    await rm(codexB, { recursive: true, force: true })
  }
})

test('createCodexProvider exposes credentials surface by default', async () => {
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-cred-prov-'))
  try {
    const provider = createCodexProvider({ stateDir, credentials: true })
    expect(provider.credentials).toBeDefined()
    expect(provider.credentials!.capability()).toMatchObject({ mode: 'supported', multi: true })
    expect(await provider.credentials!.list()).toEqual([])
  } finally {
    await rm(stateDir, { recursive: true, force: true })
  }
})

test('createCodexProvider with custom authStore omits credentials by default', async () => {
  const provider = createCodexProvider({
    authStore: {
      status: async () => ({ status: 'authenticated', accountLabel: 'static' }),
      resolveAuth: async () => {
        throw new Error('unused')
      },
    },
  })
  expect(provider.credentials).toBeUndefined()
})

test('beginLogin runs the device-code flow, surfaces pending material, and imports into the pool', async () => {
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-cred-login-'))
  const codexHome = await mkdtemp(join(tmpdir(), 'demi-codex-login-'))
  try {
    const access = jwt({
      email: 'device@example.com',
      'https://api.openai.com/auth': { chatgpt_account_id: 'acct-device' },
    })
    const id = jwt({ email: 'device@example.com' })
    let polls = 0
    const loginFetch = (async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input)
      const jsonResponse = (status: number, body: unknown) =>
        new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
      if (url.endsWith('/deviceauth/usercode')) {
        return jsonResponse(200, { device_auth_id: 'dev_1', user_code: 'CODE-0001', interval: '0' })
      }
      if (url.endsWith('/deviceauth/token')) {
        polls += 1
        if (polls === 1) return jsonResponse(403, {})
        return jsonResponse(200, { authorization_code: 'authz', code_challenge: 'c', code_verifier: 'v' })
      }
      if (url.endsWith('/oauth/token')) {
        return jsonResponse(200, { id_token: id, access_token: access, refresh_token: 'rt' })
      }
      throw new Error(`unexpected fetch: ${url}`)
    }) as typeof fetch

    const pool = openCodexCredentialPool({ stateDir })
    const authStore = new PoolAwareCodexAuthStore(pool, { codexHome })
    const credentials = createCodexCredentials(pool, authStore, { codexHome, loginFetch })

    const pendings: Array<{ verificationUrl: string; userCode: string }> = []
    const result = await credentials.beginLogin!({ onPending: (p) => pendings.push(p) })

    expect(pendings).toHaveLength(1)
    expect(pendings[0]!.verificationUrl).toBe('https://auth.openai.com/codex/device')
    expect(pendings[0]!.userCode).toBe('CODE-0001')

    expect(result.status).toBe('completed')
    if (result.status !== 'completed') throw new Error('unreachable')
    expect(result.credentialId).toBeTruthy()

    const list = await credentials.list()
    expect(list.some((c) => c.id === result.credentialId && c.label === 'device@example.com')).toBe(true)
    const active = await credentials.getActive()
    expect(active.credentialId).toBe(result.credentialId!)
  } finally {
    await rm(stateDir, { recursive: true, force: true })
    await rm(codexHome, { recursive: true, force: true })
  }
})
