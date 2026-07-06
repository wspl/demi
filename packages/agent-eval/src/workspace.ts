import { cp, mkdir, mkdtemp, readFile, readdir } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, relative, resolve } from 'node:path'

/**
 * Workspace fixtures: every attempt gets a fresh, isolated copy of the fixture
 * directory so Worker side effects never leak between runs, plus a before/after
 * content index that yields an auditable diff.
 */

export interface WorkspaceSnapshot {
  /** Relative path -> content hash-ish key (byte length + prefix) for cheap comparison. */
  files: Map<string, string>
}

export interface WorkspaceDiffEntry {
  path: string
  status: 'added' | 'removed' | 'changed'
}

export async function prepareWorkspace(fixtureSource: string | null, ignore: readonly string[]): Promise<string> {
  const workspace = await mkdtemp(join(tmpdir(), 'demi-eval-workspace-'))
  if (fixtureSource) {
    await cp(resolve(fixtureSource), workspace, {
      recursive: true,
      filter: (source) => !isIgnored(relative(resolve(fixtureSource), source), ignore),
    })
  }
  return workspace
}

export async function snapshotWorkspace(root: string, ignore: readonly string[] = []): Promise<WorkspaceSnapshot> {
  const files = new Map<string, string>()
  await indexDirectory(root, root, ignore, files)
  return { files }
}

export function diffWorkspace(before: WorkspaceSnapshot, after: WorkspaceSnapshot): WorkspaceDiffEntry[] {
  const entries: WorkspaceDiffEntry[] = []
  for (const [path, key] of after.files) {
    const previous = before.files.get(path)
    if (previous === undefined) entries.push({ path, status: 'added' })
    else if (previous !== key) entries.push({ path, status: 'changed' })
  }
  for (const path of before.files.keys()) {
    if (!after.files.has(path)) entries.push({ path, status: 'removed' })
  }
  return entries.sort((a, b) => a.path.localeCompare(b.path))
}

export function renderWorkspaceDiff(entries: WorkspaceDiffEntry[]): string {
  if (entries.length === 0) return '(no workspace changes)\n'
  return `${entries.map((entry) => `${entry.status.padEnd(8)}${entry.path}`).join('\n')}\n`
}

/**
 * Minimal glob matching for fixture ignore rules and diff path policies:
 * `**` crosses directories, `*` matches within a segment; bare directory
 * prefixes match their whole subtree.
 */
export function matchesPathPattern(path: string, pattern: string): boolean {
  if (!pattern.includes('*')) {
    return path === pattern || path.startsWith(`${pattern.replace(/\/$/, '')}/`)
  }
  const regex = new RegExp(
    `^${pattern
      .split(/(\*\*\/|\*\*|\*)/)
      .map((part) => {
        // gitignore semantics: `**/` spans zero or more directories.
        if (part === '**/') return '(?:.*/)?'
        if (part === '**') return '.*'
        if (part === '*') return '[^/]*'
        return part.replace(/[.+^${}()|[\]\\?]/g, '\\$&')
      })
      .join('')}$`,
  )
  return regex.test(path)
}

function isIgnored(relativePath: string, ignore: readonly string[]): boolean {
  if (!relativePath) return false
  const normalized = relativePath.split('\\').join('/')
  return ignore.some((pattern) => matchesPathPattern(normalized, pattern))
}

async function indexDirectory(
  root: string,
  directory: string,
  ignore: readonly string[],
  files: Map<string, string>,
): Promise<void> {
  let entries
  try {
    entries = await readdir(directory, { withFileTypes: true })
  } catch {
    return
  }
  for (const entry of entries) {
    const full = join(directory, entry.name)
    const rel = relative(root, full).split('\\').join('/')
    if (isIgnored(rel, ignore)) continue
    if (entry.isDirectory()) {
      await indexDirectory(root, full, ignore, files)
      continue
    }
    if (!entry.isFile()) continue
    try {
      const content = await readFile(full)
      files.set(rel, contentKey(content))
    } catch {
      // Unreadable files are treated as absent.
    }
  }
}

function contentKey(content: Uint8Array): string {
  let hash = 2166136261
  for (let i = 0; i < content.byteLength; i += 1) {
    hash ^= content[i]!
    hash = Math.imul(hash, 16777619)
  }
  return `${content.byteLength}:${(hash >>> 0).toString(16)}`
}

export async function ensureDirectory(path: string): Promise<void> {
  await mkdir(path, { recursive: true })
}
