import { afterEach, describe, expect, it } from 'bun:test'
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  CredentialPoolError,
  FileCredentialPool
} from '../credentials-pool'

const stateDirs: string[] = []

async function newPool(): Promise<FileCredentialPool> {
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-credential-pool-'))
  stateDirs.push(stateDir)
  return new FileCredentialPool({
    stateDir,
    providerKey: 'test-provider',
    secretFileName: 'auth.json',
  })
}

/** Writes a raw `meta.json`, bypassing the pool's own validation on write. */
async function writeRawMeta(
  pool: FileCredentialPool,
  id: string,
  meta: unknown
): Promise<void> {
  await mkdir(pool.entryDir(id), { recursive: true })
  await writeFile(pool.metaPath(id), JSON.stringify(meta), 'utf8')
}

afterEach(async () => {
  for (const dir of stateDirs.splice(0)) await rm(dir, {
    recursive: true,
    force: true
  })
})

describe('FileCredentialPool.readMeta', () => {
  it('returns null when the entry does not exist', async () => {
    const pool = await newPool()
    expect(await pool.readMeta('cred-missing')).toBeNull()
  })

  it('reads back what writeEntry stored', async () => {
    const pool = await newPool()
    const meta = {
      id: 'cred-1',
      label: 'account@example.com',
      detail: 'oidc',
      updatedAt: new Date(1_700_000_000_000).toISOString(),
      source: 'import',
      identityKey: 'email:account@example.com',
    }
    await pool.writeEntry(meta, '{"token":"secret"}\n')
    expect(await pool.readMeta('cred-1')).toEqual(meta)
  })

  it('rejects metadata without a label', async () => {
    const pool = await newPool()
    await writeRawMeta(pool, 'cred-1', {
      id: 'cred-1',
      updatedAt: new Date(0).toISOString(),
    })
    await expect(pool.readMeta('cred-1')).rejects.toThrow(CredentialPoolError)
  })

  it('rejects a numeric updatedAt instead of substituting a date', async () => {
    const pool = await newPool()
    await writeRawMeta(pool, 'cred-1', {
      id: 'cred-1',
      label: 'account',
      updatedAt: 1_700_000_000_000,
    })
    await expect(pool.readMeta('cred-1')).rejects.toThrow(/updatedAt/)
  })

  it('rejects metadata that is not JSON', async () => {
    const pool = await newPool()
    await mkdir(pool.entryDir('cred-1'), { recursive: true })
    await writeFile(pool.metaPath('cred-1'), 'not json', 'utf8')
    await expect(pool.readMeta('cred-1')).rejects.toThrow(/not JSON/)
  })

  it('skips a stray file sitting beside the entry directories', async () => {
    const pool = await newPool()
    await mkdir(pool.entriesDir(), { recursive: true })
    await writeFile(join(pool.entriesDir(), '.DS_Store'), 'junk', 'utf8')
    expect(await pool.listMeta()).toEqual([])
  })

  it('lists nothing before any entry is written', async () => {
    const pool = await newPool()
    expect(await pool.list()).toEqual([])
    expect(await pool.getActiveId()).toBeNull()
  })
})

it('a file pool document stores a refreshed secret only over the revision it was read at', async () => {
  const { mkdtemp, readFile, rm, stat } = await import('node:fs/promises')
  const { tmpdir } = await import('node:os')
  const { join } = await import('node:path')
  const { FileCredentialPool } = await import('../credentials-pool')
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-pool-document-'))
  try {
    const pool = new FileCredentialPool({
      stateDir,
      providerKey: 'vendor',
      secretFileName: 'auth.json'
    })
    expect(await pool.document('absent').read()).toBeNull()
    await pool.writeEntry(
      { id: 'a', label: 'a@example.com', updatedAt: new Date().toISOString() },
      '{"refresh":"one"}'
    )
    const first = await pool.document('a').read()
    const second = await pool.document('a').read()
    expect(first?.text).toBe('{"refresh":"one"}')
    expect(await pool.document('a').replace('{"refresh":"two"}', first!.version)).toBe(true)
    expect(await pool.document('a').replace('{"refresh":"lost"}', second!.version)).toBe(false)
    expect(await readFile(pool.secretPath('a'), 'utf8')).toBe('{"refresh":"two"}')
    expect((await stat(pool.secretPath('a'))).mode & 0o777).toBe(0o600)
  } finally {
    await rm(stateDir, { recursive: true, force: true })
  }
})

it('a composed pool is the fallback while the primary holds no account, and the primary once it does', async () => {
  const {
    fallbackCredentialPool,
    MemoryCredentialPool
  } = await import('../credentials-pool')
  const meta = (id: string) => ({
    id,
    label: `${id}@example.com`,
    identityKey: id,
    updatedAt: new Date().toISOString()
  })
  const primary = new MemoryCredentialPool()
  const fallback = new MemoryCredentialPool()
  await fallback.writeEntry(meta('vendor'), 'vendor-secret')
  const pool = fallbackCredentialPool(primary, fallback)

  expect((await pool.list()).map(account => account.id)).toEqual(['vendor'])
  expect(await pool.ensureActivePointer()).toBe('vendor')
  expect((await pool.document('vendor').read())?.text).toBe('vendor-secret')
  // A refresh of the fallback's account is kept by the fallback.
  const revision = (await pool.document('vendor').read())!
  expect(await pool.document('vendor').replace('vendor-renewed', revision.version)).toBe(true)
  expect((await fallback.document('vendor').read())?.text).toBe('vendor-renewed')

  // Writes go to the primary, whose first account takes over.
  await pool.writeEntry(meta('own'), 'own-secret')
  expect(primary.entries().map(entry => entry.meta.id)).toEqual(['own'])
  expect((await pool.list()).map(account => account.id)).toEqual(['own'])
  expect(await pool.ensureActivePointer()).toBe('own')
  expect((await pool.document('own').read())?.text).toBe('own-secret')
  await pool.remove('own')
  expect((await pool.list()).map(account => account.id)).toEqual(['vendor'])
})

it('work queued under one key takes turns, and a failure does not stop the next', async () => {
  const { queuedExclusive } = await import('../credentials-pool')
  const order: string[] = []
  const step = (name: string, fail = false) => async () => {
    order.push(`${name}:start`)
    await new Promise(resolve => setTimeout(resolve, 5))
    order.push(`${name}:end`)
    if (fail)
      throw new Error(name)
    return name
  }
  const results = await Promise.allSettled([
    queuedExclusive('k')(step('a', true)),
    queuedExclusive('k')(step('b')),
    queuedExclusive('other')(step('c')),
  ])
  expect(results.map(result => result.status)).toEqual(['rejected', 'fulfilled', 'fulfilled'])
  expect(order.indexOf('a:end')).toBeLessThan(order.indexOf('b:start'))
  expect(order.indexOf('c:start')).toBeLessThan(order.indexOf('a:end'))
})
