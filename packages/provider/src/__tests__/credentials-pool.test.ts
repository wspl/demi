import { afterEach, expect, test } from 'bun:test'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { CredentialPoolError, FileCredentialPool } from '../credentials-pool'

const directories: string[] = []
afterEach(async () => {
  for (const directory of directories.splice(0)) {
    await rm(directory, { recursive: true, force: true })
  }
})

async function poolFixture() {
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-contract-pool-'))
  directories.push(stateDir)
  return new FileCredentialPool({ stateDir, providerKey: 'fixture', secretFileName: 'secret.json' })
}

const metadata = {
  id: 'entry-a',
  label: 'Test account',
  detail: null,
  updatedAt: '2026-09-12T00:00:00.000Z',
}

async function putRawMeta(pool: FileCredentialPool, value: unknown) {
  await mkdir(pool.entryDir(metadata.id), { recursive: true })
  await writeFile(pool.metaPath(metadata.id), JSON.stringify(value))
}

test('metadata round trips without filling or rewriting optional values', async () => {
  const pool = await poolFixture()
  await pool.writeEntry(metadata, 'synthetic-secret')
  expect(await pool.readMeta(metadata.id)).toEqual(metadata)
  expect(await pool.listMeta()).toEqual([metadata])
  await pool.setActiveId(metadata.id)
  expect(await pool.getActiveId()).toBe(metadata.id)
  expect(await pool.readSecretText(metadata.id)).toBe('synthetic-secret')
})

test('only missing files and directories produce absent results', async () => {
  const pool = await poolFixture()
  expect(await pool.readMeta(metadata.id)).toBeNull()
  expect(await pool.getActiveId()).toBeNull()
  expect(await pool.listMeta()).toEqual([])
  await mkdir(pool.root, { recursive: true })
  await writeFile(pool.entriesDir(), 'not a directory')
  await expect(pool.listMeta()).rejects.toMatchObject({ code: 'ENOTDIR' })
  await rm(pool.entriesDir())
  await mkdir(pool.metaPath(metadata.id), { recursive: true })
  await expect(pool.readMeta(metadata.id)).rejects.toMatchObject({ code: 'EISDIR' })
  await mkdir(pool.activePath())
  await expect(pool.getActiveId()).rejects.toMatchObject({ code: 'EISDIR' })
})

test('invalid metadata cannot become an empty or repaired credential', async () => {
  const pool = await poolFixture()
  for (const value of [
    { label: 'test', updatedAt: 42 },
    { ...metadata, id: 'different-id' },
    { ...metadata, updatedAt: 'yesterday' },
    { ...metadata, detail: 42 },
    { ...metadata, source: '' },
    { ...metadata, label: '  ' },
    { ...metadata, unexpected: true },
    [], null,
  ]) {
    await putRawMeta(pool, value)
    await expect(pool.readMeta(metadata.id)).rejects.toMatchObject({ code: 'credential_invalid' })
    await expect(pool.listMeta()).rejects.toBeInstanceOf(CredentialPoolError)
  }
  await writeFile(pool.metaPath(metadata.id), 'TOP_SECRET_BAD_JSON')
  try {
    await pool.readMeta(metadata.id)
    throw new Error('expected invalid metadata')
  } catch (error) {
    expect(error).toBeInstanceOf(CredentialPoolError)
    expect(String(error)).not.toContain('TOP_SECRET')
  }
  await rm(pool.metaPath(metadata.id))
  await expect(pool.listMeta()).rejects.toMatchObject({ code: 'credential_invalid' })
})

test('invalid pointers cannot trigger another account selection', async () => {
  const pool = await poolFixture()
  await pool.writeEntry(metadata, 'synthetic-secret')
  for (const pointer of ['', ' entry-a\n', 'entry-a\n\n', '../other', 'missing-id\n']) {
    await writeFile(pool.activePath(), pointer)
    await expect(pool.ensureActivePointer()).rejects.toMatchObject({ code: 'credential_invalid' })
    expect(await readFile(pool.activePath(), 'utf8')).toBe(pointer)
  }
  await pool.clearActive()
  expect(await pool.ensureActivePointer()).toBe(metadata.id)
})

test('explicit deletion can remove corrupt metadata and clears its valid pointer', async () => {
  const pool = await poolFixture()
  await pool.writeEntry(metadata, 'synthetic-secret')
  await pool.setActiveId(metadata.id)
  await putRawMeta(pool, { label: 'broken' })
  await expect(pool.getActiveId()).rejects.toMatchObject({ code: 'credential_invalid' })
  await pool.remove(metadata.id)
  expect(await pool.getActiveId()).toBeNull()
  expect(await pool.listMeta()).toEqual([])
})

test('missing secret and unreadable secret are different failures', async () => {
  const pool = await poolFixture()
  await putRawMeta(pool, metadata)
  await expect(pool.setActiveId(metadata.id)).rejects.toMatchObject({ code: 'credential_not_found' })
  await mkdir(pool.secretPath(metadata.id))
  await expect(pool.setActiveId(metadata.id)).rejects.toMatchObject({ code: 'EISDIR' })
  await expect(pool.readMeta('../outside')).rejects.toMatchObject({ code: 'credential_invalid' })
})
