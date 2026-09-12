import { expect, test } from 'bun:test'
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { resolveGrokClientVersion } from '../headers'

test('optional CLI version metadata validates the consumed field and falls back as a whole', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'demi-grok-version-'))
  const path = join(dir, 'version.json')
  try {
    const fallback = resolveGrokClientVersion(undefined, dir)
    expect(fallback).toBe('1.0.5')
    await writeFile(path, JSON.stringify({ version: '2.3.4', other: true }))
    expect(resolveGrokClientVersion(undefined, dir)).toBe('2.3.4')
    expect(resolveGrokClientVersion('synthetic-client', dir)).toBe('synthetic-client')
    for (const value of ['{', 'null', '[]', '{"version":3}', '{"version":" "}']) {
      await writeFile(path, value)
      expect(resolveGrokClientVersion(undefined, dir)).toBe(fallback)
    }
    await rm(path)
    await mkdir(path)
    expect(resolveGrokClientVersion(undefined, dir)).toBe(fallback)
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})
