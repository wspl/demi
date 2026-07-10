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
