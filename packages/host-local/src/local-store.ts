import type { Dirent } from 'node:fs'
import { mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import { dirname, relative, resolve } from 'node:path'
import type { HostStore } from '@demi/shell'

export class LocalHostStore implements HostStore {
  readonly root: string

  constructor(root: string) {
    this.root = resolve(root)
  }

  async readJson<T>(key: string): Promise<T | null> {
    try {
      const content = await readFile(this.pathForKey(key), 'utf8')
      return JSON.parse(content) as T
    } catch (error) {
      if (isNotFound(error)) return null
      throw error
    }
  }

  async writeJson<T>(key: string, value: T): Promise<void> {
    const path = this.pathForKey(key)
    await mkdir(dirname(path), { recursive: true })
    await writeFile(path, JSON.stringify(value, null, 2), 'utf8')
  }

  async delete(key: string): Promise<void> {
    await rm(this.pathForKey(key), { force: true, recursive: true })
  }

  async list(prefix: string): Promise<string[]> {
    const start = this.pathForKey(prefix || '.')
    const found: string[] = []
    await collectJsonFiles(start, this.root, found)
    return found.sort()
  }

  private pathForKey(key: string): string {
    validateHostStoreKey(key)
    const path = resolve(this.root, key)
    const rel = relative(this.root, path)
    if (rel.startsWith('..') || rel === '..' || path === this.root) {
      if (key === '' || key === '.') return this.root
      throw new Error(`Invalid HostStore key: ${key}`)
    }
    return path
  }
}

async function collectJsonFiles(path: string, root: string, found: string[]): Promise<void> {
  let entries: Dirent[]
  try {
    entries = await readdir(path, { withFileTypes: true })
  } catch (error) {
    if (isNotFound(error)) return
    throw error
  }

  for (const entry of entries) {
    const fullPath = resolve(path, entry.name)
    if (entry.isDirectory()) {
      await collectJsonFiles(fullPath, root, found)
    } else {
      found.push(relative(root, fullPath))
    }
  }
}

function isNotFound(error: unknown): boolean {
  return error instanceof Error && 'code' in error && error.code === 'ENOENT'
}

function validateHostStoreKey(key: string): void {
  if (key === '' || key === '.') return
  if (key.includes('\0')) throw new Error(`Invalid HostStore key: ${key}`)
  if (key.startsWith('/') || /^[A-Za-z]:[\\/]/.test(key)) {
    throw new Error(`HostStore keys must be relative: ${key}`)
  }
  for (const segment of key.split(/[\\/]+/)) {
    if (segment === '..') throw new Error(`HostStore keys must not contain path traversal: ${key}`)
  }
}
