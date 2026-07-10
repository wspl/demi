import { asError, bytesToBase64, decodeUtf8, dirnamePath, encodeUtf8, errorMessage } from '@demicodes/utils'
import { z } from 'zod'
import type { Command, CommandAsset, Host } from '@demicodes/shell'

export function createDemiCommand(host: Host): Command {
  return {
    name: 'demi',
    summary: 'Read, create, edit, and patch workspace files (text, images, and native video).',
    subcommands: [
      {
        name: 'read',
        summary: 'Read a file. Text is returned as text; images and supported videos are returned as viewable media.',
        effects: 'reads one file; does not modify anything',
        successOutput: 'prints text file contents to stdout, or returns an image/video file as viewable media',
        failureOutput: 'writes the reason to stderr and exits non-zero if the path is missing or unreadable',
        input: {
          path: z.string().describe('File path to read'),
        },
        positionals: ['path'],
        examples: ['demi read src/foo.ts', 'demi read assets/frame.png', 'demi read assets/clip.mp4'],
        run: async ({ parsed, cwd, io, supportedAssetTypes }) => {
          const path = String(parsed.values.path)
          const pathError = pathValidationError(path)
          if (pathError) {
            await io.stderr(`${pathError}\n`)
            return { exitCode: 1 }
          }

          const media = mediaForPath(path)
          if (media) {
            if (media.kind === 'video' && !supportedAssetTypes.has('video')) {
              await io.stderr('Current model does not support video input.\n')
              return { exitCode: 1 }
            }
            let bytes: Uint8Array
            try {
              bytes = await host.fs.readFile(path, { cwd })
            } catch (error) {
              await io.stderr(`${errorMessage(error)}\n`)
              return { exitCode: 1 }
            }
            const asset: CommandAsset = { type: media.kind, mediaType: media.mediaType, data: bytesToBase64(bytes) }
            await io.asset(asset)
            await io.stdout(`Read ${media.kind} ${path} (${media.mediaType}, ${bytes.length} bytes)\n`)
            return { exitCode: 0 }
          }

          const read = await readFile(host, cwd, path)
          if (read.exitCode !== 0) {
            await io.stderr(read.stderr)
            return { exitCode: read.exitCode }
          }
          await io.stdout(read.stdout)
          return { exitCode: 0 }
        },
      },
      {
        name: 'create',
        summary: 'Create a new file. Fails if the file exists.',
        effects: 'modifies files by creating a new file; does not modify command storage',
        successOutput: 'writes "Created <path>" to stdout',
        failureOutput: 'writes the reason to stderr and exits non-zero without overwriting existing files',
        input: {
          path: z.string().describe('Target file path'),
          content: z.string().describe('File content, passed via stdin/heredoc'),
        },
        positionals: ['path'],
        stdinField: 'content',
        examples: [
          "demi create src/foo.ts <<'EOF'\nexport const foo = 1\nEOF",
          "demi create README.md <<'EOF'\n# Project\nEOF",
        ],
        run: async ({ parsed, cwd, io }) => {
          const path = String(parsed.values.path)
          const content = String(parsed.values.content)
          const pathError = pathValidationError(path)
          if (pathError) {
            await io.stderr(`${pathError}\n`)
            return { exitCode: 1 }
          }
          if (await exists(host, cwd, path)) {
            await io.stderr(`File already exists: ${path}\n`)
            return { exitCode: 1 }
          }
          const parent = dirnamePath(path)
          if (parent !== '.') {
            const made = await mkdirPath(host, cwd, parent)
            if (made.exitCode !== 0) {
              await io.stderr(made.stderr)
              return { exitCode: made.exitCode }
            }
          }
          const write = await writeFile(host, cwd, path, content)
          if (write.exitCode !== 0) {
            await io.stderr(write.stderr)
            return { exitCode: write.exitCode }
          }
          await io.stdout(`Created ${path}\n`)
          return {
            exitCode: 0,
            metadata: fileDiffsMetadata([fileDiffMetadata('create', null, path, '', content)]),
          }
        },
      },
      {
        name: 'edit',
        summary: 'Replace exact text in an existing file.',
        effects: 'modifies one existing file; does not modify command storage',
        successOutput: 'writes "Edited <path>" to stdout',
        failureOutput: 'writes no-match, ambiguous-match, or write errors to stderr and exits non-zero without partial writes',
        input: {
          path: z.string().describe('Target file path'),
          old: z.string().describe('Exact text to replace'),
          new: z.string().describe('Replacement text'),
          occurrence: z.number().int().positive().optional().describe('1-based occurrence to replace'),
          context: z.number().int().positive().optional().describe('Line number used to choose nearest occurrence'),
        },
        positionals: ['path'],
        examples: [
          'demi edit src/foo.ts --old foo --new bar',
          'demi edit src/foo.ts --old "old text" --new "new text" --occurrence 2',
        ],
        run: async ({ parsed, cwd, io }) => {
          const path = String(parsed.values.path)
          const read = await readFile(host, cwd, path)
          if (read.exitCode !== 0) {
            await io.stderr(read.stderr)
            return { exitCode: read.exitCode }
          }

          const oldText = String(parsed.values.old)
          const newText = String(parsed.values.new)
          if (oldText.length === 0) {
            await io.stderr('Old text must not be empty\n')
            return { exitCode: 1 }
          }
          const matches = findMatches(read.stdout, oldText)
          if (matches.length === 0) {
            await io.stderr(`No match found in ${path}\n`)
            return { exitCode: 1 }
          }

          const selection = chooseMatch(matches, parsed.values.occurrence, parsed.values.context)
          if (!selection.match) {
            await io.stderr(`${selection.reason} in ${path}: ${formatMatches(matches)}\n`)
            return { exitCode: 1 }
          }

          const match = selection.match
          const content = `${read.stdout.slice(0, match.index)}${newText}${read.stdout.slice(match.index + oldText.length)}`
          const write = await writeFile(host, cwd, path, content)
          if (write.exitCode !== 0) {
            await io.stderr(write.stderr)
            return { exitCode: write.exitCode }
          }
          await io.stdout(`Edited ${path}\n`)
          return {
            exitCode: 0,
            metadata: fileDiffsMetadata([fileDiffMetadata('edit', path, path, read.stdout, content)]),
          }
        },
      },
      {
        name: 'patch',
        summary: 'Apply a unified diff patch to one or more files.',
        effects: 'modifies files described by the patch; validates all patch operations before writing',
        successOutput: 'writes "Patched <n> file(s)" to stdout',
        failureOutput: 'writes parse, validation, or write errors to stderr and exits non-zero after rolling back partial writes when possible',
        input: {
          patch: z.string().describe('Unified diff content, passed via stdin/heredoc'),
        },
        stdinField: 'patch',
        examples: [
          "demi patch <<'PATCH'\n--- a/src/foo.ts\n+++ b/src/foo.ts\n@@ -1 +1 @@\n-old\n+new\nPATCH",
          "demi patch <<'PATCH'\n--- /dev/null\n+++ b/src/new.ts\n@@ -0,0 +1 @@\n+export const created = true\nPATCH",
        ],
        run: async ({ parsed, cwd, io }) => {
          let patches: FilePatch[]
          let operations: PatchOperation[]
          try {
            patches = parseUnifiedDiff(String(parsed.values.patch))
            operations = await planPatchOperations(host, cwd, patches)
          } catch (error) {
            await io.stderr(`${asError(error).message}\n`)
            return { exitCode: 1 }
          }

          const applied = await applyPatchOperations(host, cwd, operations)
          if (applied.exitCode !== 0) {
            await io.stderr(applied.stderr)
            return { exitCode: applied.exitCode }
          }
          await io.stdout(`Patched ${patches.length} file(s)\n`)
          return {
            exitCode: 0,
            metadata: fileDiffsMetadata(operations.map(operationDiffMetadata)),
          }
        },
      },
    ],
  }
}

interface ProcessResult {
  stdout: string
  stderr: string
  exitCode: number
}

async function exists(host: Host, cwd: string, path: string): Promise<boolean> {
  if (pathValidationError(path)) return false
  return host.fs.exists(path, { cwd })
}

async function readFile(host: Host, cwd: string, path: string): Promise<ProcessResult> {
  const pathError = pathValidationError(path)
  if (pathError) return { stdout: '', stderr: `${pathError}\n`, exitCode: 1 }
  try {
    return { stdout: decodeUtf8(await host.fs.readFile(path, { cwd })), stderr: '', exitCode: 0 }
  } catch (error) {
    return { stdout: '', stderr: `${errorMessage(error)}\n`, exitCode: 1 }
  }
}

async function writeFile(host: Host, cwd: string, path: string, content: string): Promise<ProcessResult> {
  const pathError = pathValidationError(path)
  if (pathError) return { stdout: '', stderr: `${pathError}\n`, exitCode: 1 }
  try {
    await host.fs.writeFile(path, encodeUtf8(content), { cwd })
    return { stdout: '', stderr: '', exitCode: 0 }
  } catch (error) {
    return { stdout: '', stderr: `${errorMessage(error)}\n`, exitCode: 1 }
  }
}

async function deleteFile(host: Host, cwd: string, path: string): Promise<ProcessResult> {
  const pathError = pathValidationError(path)
  if (pathError) return { stdout: '', stderr: `${pathError}\n`, exitCode: 1 }
  try {
    await host.fs.rm(path, { cwd, force: true })
    return { stdout: '', stderr: '', exitCode: 0 }
  } catch (error) {
    return { stdout: '', stderr: `${errorMessage(error)}\n`, exitCode: 1 }
  }
}

async function mkdirPath(host: Host, cwd: string, path: string): Promise<ProcessResult> {
  const pathError = pathValidationError(path)
  if (pathError) return { stdout: '', stderr: `${pathError}\n`, exitCode: 1 }
  try {
    await host.fs.mkdir(path, { cwd, recursive: true })
    return { stdout: '', stderr: '', exitCode: 0 }
  } catch (error) {
    return { stdout: '', stderr: `${errorMessage(error)}\n`, exitCode: 1 }
  }
}

interface Match {
  index: number
  line: number
}

interface MatchSelection {
  match: Match | null
  reason: string
}

function findMatches(content: string, search: string): Match[] {
  const matches: Match[] = []
  let index = content.indexOf(search)
  while (index !== -1) {
    matches.push({ index, line: content.slice(0, index).split('\n').length })
    index = content.indexOf(search, index + search.length)
  }
  return matches
}

function chooseMatch(matches: Match[], occurrence: unknown, context: unknown): MatchSelection {
  if (typeof occurrence === 'number') {
    const match = matches[occurrence - 1] ?? null
    return {
      match,
      reason: match ? '' : `Occurrence ${occurrence} is out of range`,
    }
  }
  if (typeof context === 'number') {
    const ranked = matches
      .map((match) => ({ match, distance: Math.abs(match.line - context) }))
      .sort((a, b) => a.distance - b.distance)
    const nearest = ranked[0]
    if (!nearest) return { match: null, reason: 'No match found' }
    const tied = ranked.filter((entry) => entry.distance === nearest.distance)
    return tied.length === 1
      ? { match: nearest.match, reason: '' }
      : { match: null, reason: `Context line ${context} is ambiguous` }
  }
  return matches.length === 1
    ? { match: matches[0], reason: '' }
    : { match: null, reason: 'Multiple matches' }
}

function formatMatches(matches: Match[]): string {
  return matches.map((match, index) => `occurrence ${index + 1} at line ${match.line}`).join(', ')
}

interface FilePatch {
  oldPath: string | null
  newPath: string | null
  hunks: Hunk[]
}

interface Hunk {
  oldStart: number
  lines: PatchLine[]
}

type PatchLine = { kind: 'context' | 'remove' | 'add'; text: string; noNewline?: boolean }

type PatchOperation =
  | {
      type: 'write'
      path: string
      content: string
      original: string
      oldPath: string | null
      newPath: string
      deletePath?: string
    }
  | { type: 'delete'; path: string; original: string; oldPath: string; newPath: null }

type RollbackAction = { type: 'write'; path: string; content: string } | { type: 'delete'; path: string }

async function applyPatchOperations(host: Host, cwd: string, operations: PatchOperation[]): Promise<ProcessResult> {
  const rollback: RollbackAction[] = []

  const fail = async (failure: ProcessResult): Promise<ProcessResult> => {
    const rollbackFailures: ProcessResult[] = []
    for (let index = rollback.length - 1; index >= 0; index -= 1) {
      const result = await runRollbackAction(host, cwd, rollback[index])
      if (result.exitCode !== 0) rollbackFailures.push(result)
    }
    return appendRollbackFailures(failure, rollbackFailures)
  }

  for (const operation of operations) {
    if (operation.type === 'delete') {
      rollback.push({ type: 'write', path: operation.path, content: operation.original })
      const deleted = await deleteFile(host, cwd, operation.path)
      if (deleted.exitCode !== 0) return fail(deleted)
      continue
    }

    const parent = dirnamePath(operation.path)
    if (parent !== '.') {
      const made = await mkdirPath(host, cwd, parent)
      if (made.exitCode !== 0) return fail(made)
    }

    rollback.push(rollbackForWrite(operation))
    const write = await writeFile(host, cwd, operation.path, operation.content)
    if (write.exitCode !== 0) return fail(write)

    if (operation.deletePath) {
      rollback.push({ type: 'write', path: operation.deletePath, content: operation.original })
      const deleted = await deleteFile(host, cwd, operation.deletePath)
      if (deleted.exitCode !== 0) return fail(deleted)
    }
  }

  return { stdout: '', stderr: '', exitCode: 0 }
}

function rollbackForWrite(operation: Extract<PatchOperation, { type: 'write' }>): RollbackAction {
  if (operation.oldPath === operation.path) {
    return { type: 'write', path: operation.path, content: operation.original }
  }
  return { type: 'delete', path: operation.path }
}

async function runRollbackAction(host: Host, cwd: string, action: RollbackAction): Promise<ProcessResult> {
  if (action.type === 'delete') return deleteFile(host, cwd, action.path)
  const parent = dirnamePath(action.path)
  if (parent !== '.') {
    const made = await mkdirPath(host, cwd, parent)
    if (made.exitCode !== 0) return made
  }
  return writeFile(host, cwd, action.path, action.content)
}

function appendRollbackFailures(failure: ProcessResult, rollbackFailures: ProcessResult[]): ProcessResult {
  if (rollbackFailures.length === 0) return failure
  const rollbackStderr = rollbackFailures
    .map((result) => result.stderr.trim() || `rollback command exited ${result.exitCode}`)
    .join('\n')
  const separator = failure.stderr.length === 0 || failure.stderr.endsWith('\n') ? '' : '\n'
  return {
    ...failure,
    stderr: `${failure.stderr}${separator}Rollback failed:\n${rollbackStderr}\n`,
  }
}

function parseUnifiedDiff(diff: string): FilePatch[] {
  const lines = diff.split('\n')
  const patches: FilePatch[] = []
  let current: FilePatch | null = null
  let hunk: Hunk | null = null
  let pendingOldPath: string | null | undefined

  for (const line of lines) {
    if (line.startsWith('--- ')) {
      pendingOldPath = parseDiffPath(line.slice(4))
      hunk = null
      continue
    }
    if (line.startsWith('+++ ')) {
      if (pendingOldPath === undefined) throw new Error('Invalid patch: new file header before old file header')
      current = { oldPath: pendingOldPath, newPath: parseDiffPath(line.slice(4)), hunks: [] }
      patches.push(current)
      pendingOldPath = undefined
      continue
    }
    if (line.startsWith('@@ ')) {
      if (!current) throw new Error('Invalid patch: hunk before file header')
      const match = /^@@ -(\d+)(?:,\d+)? \+\d+(?:,\d+)? @@/.exec(line)
      if (!match) throw new Error(`Invalid patch hunk header: ${line}`)
      hunk = { oldStart: Number(match[1]), lines: [] }
      current.hunks.push(hunk)
      continue
    }
    if (!hunk) continue
    if (line.startsWith(' ')) hunk.lines.push({ kind: 'context', text: line.slice(1) })
    else if (line.startsWith('-')) hunk.lines.push({ kind: 'remove', text: line.slice(1) })
    else if (line.startsWith('+')) hunk.lines.push({ kind: 'add', text: line.slice(1) })
    else if (line === '\\ No newline at end of file') {
      const previous = hunk.lines[hunk.lines.length - 1]
      if (previous) previous.noNewline = true
    }
    else if (line !== '') throw new Error(`Invalid patch line: ${line}`)
  }

  if (patches.length === 0) throw new Error('Invalid patch: no files')
  for (const patch of patches) {
    if (!patch.oldPath && !patch.newPath) throw new Error('Invalid patch: both file paths are /dev/null')
    if (patch.hunks.length === 0) throw new Error(`Invalid patch: ${patchDisplayPath(patch)} has no hunks`)
  }
  return patches
}

async function planPatchOperations(host: Host, cwd: string, patches: FilePatch[]): Promise<PatchOperation[]> {
  const operations: PatchOperation[] = []

  for (const patch of patches) {
    const targetPath = patch.newPath ?? patch.oldPath
    if (!targetPath) throw new Error('Invalid patch: missing target path')
    assertValidPath(targetPath)

    const isCreate = patch.oldPath === null
    const isDelete = patch.newPath === null
    let original = ''

    if (isCreate) {
      if (await exists(host, cwd, targetPath)) throw new Error(`File already exists: ${targetPath}`)
    } else {
      const oldPath = patch.oldPath
      if (oldPath === null) throw new Error('Invalid patch: missing old path')
      assertValidPath(oldPath)
      const read = await readFile(host, cwd, oldPath)
      if (read.exitCode !== 0) throw new Error(read.stderr.trim() || `Failed to read ${oldPath}`)
      original = read.stdout
    }

    if (patch.newPath) assertValidPath(patch.newPath)
    if (patch.oldPath && patch.newPath && patch.oldPath !== patch.newPath && (await exists(host, cwd, patch.newPath))) {
      throw new Error(`File already exists: ${patch.newPath}`)
    }

    const applied = applyFilePatch(original, patch)
    if (isDelete) {
      if (applied !== '') throw new Error(`Delete patch leaves content in ${targetPath}`)
      operations.push({ type: 'delete', path: targetPath, original, oldPath: targetPath, newPath: null })
    } else {
      const deletePath = patch.oldPath && patch.newPath && patch.oldPath !== patch.newPath ? patch.oldPath : undefined
      operations.push({
        type: 'write',
        path: targetPath,
        content: applied,
        original,
        oldPath: patch.oldPath,
        newPath: targetPath,
        deletePath,
      })
    }
  }

  return operations
}

function applyFilePatch(content: string, patch: FilePatch): string {
  const file = splitTextFile(content)
  const lines = file.lines
  let finalNewline = file.finalNewline
  let offset = 0

  for (const hunk of patch.hunks) {
    const index = hunk.oldStart === 0 ? 0 : hunk.oldStart - 1 + offset
    const oldLines = hunk.lines.filter((line) => line.kind !== 'add').map((line) => line.text)
    const newLines = hunk.lines.filter((line) => line.kind !== 'remove').map((line) => line.text)
    const actual = lines.slice(index, index + oldLines.length)
    if (!arraysEqual(actual, oldLines)) {
      throw new Error(`Patch does not apply to ${patchDisplayPath(patch)} at line ${hunk.oldStart}`)
    }
    const touchesEof = index + oldLines.length === lines.length
    lines.splice(index, oldLines.length, ...newLines)
    offset += newLines.length - oldLines.length
    if (touchesEof) {
      const lastNewLine = [...hunk.lines].reverse().find((line) => line.kind !== 'remove')
      if (lastNewLine) finalNewline = !lastNewLine.noNewline
      else if (lines.length === 0) finalNewline = false
    }
  }

  return joinTextFile(lines, finalNewline)
}

function parseDiffPath(rawPath: string): string | null {
  const path = stripDiffPathMetadata(rawPath)
  if (path === '/dev/null') return null
  if (path.startsWith('a/') || path.startsWith('b/')) return path.slice(2)
  return path
}

function assertValidPath(path: string): void {
  const error = pathValidationError(path)
  if (error) throw new Error(error)
}

function pathValidationError(path: string): string | null {
  if (path.includes('\0')) return `Path contains NUL byte: ${path}`
  return null
}

// `demi read` returns images/videos as native viewable blocks; everything else as text.
// (Video only actually reaches models whose catalog marks video support; others reject it.)
const MEDIA_TYPES: Record<string, { kind: 'image' | 'video'; mediaType: string }> = {
  png: { kind: 'image', mediaType: 'image/png' },
  jpg: { kind: 'image', mediaType: 'image/jpeg' },
  jpeg: { kind: 'image', mediaType: 'image/jpeg' },
  webp: { kind: 'image', mediaType: 'image/webp' },
  gif: { kind: 'image', mediaType: 'image/gif' },
  mp4: { kind: 'video', mediaType: 'video/mp4' },
  mov: { kind: 'video', mediaType: 'video/quicktime' },
  webm: { kind: 'video', mediaType: 'video/webm' },
  m4v: { kind: 'video', mediaType: 'video/x-m4v' },
}

function mediaForPath(path: string): { kind: 'image' | 'video'; mediaType: string } | null {
  const ext = path.slice(path.lastIndexOf('.') + 1).toLowerCase()
  return MEDIA_TYPES[ext] ?? null
}

function stripDiffPathMetadata(rawPath: string): string {
  const trimmed = rawPath.trim()
  const tabIndex = trimmed.indexOf('\t')
  if (tabIndex !== -1) return trimmed.slice(0, tabIndex)
  return trimmed.replace(/\s+\d{4}-\d{2}-\d{2}(?:[ T]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:\s+[+-]\d{4})?)?$/, '')
}

function patchDisplayPath(patch: FilePatch): string {
  return patch.newPath ?? patch.oldPath ?? '<unknown>'
}

function splitTextFile(content: string): { lines: string[]; finalNewline: boolean } {
  if (content === '') return { lines: [], finalNewline: false }
  const finalNewline = content.endsWith('\n')
  const body = finalNewline ? content.slice(0, -1) : content
  return { lines: body === '' ? [] : body.split('\n'), finalNewline }
}

function joinTextFile(lines: string[], finalNewline: boolean): string {
  if (lines.length === 0) return ''
  return `${lines.join('\n')}${finalNewline ? '\n' : ''}`
}

function arraysEqual(left: string[], right: string[]): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index])
}

interface FileDiffsMetadata {
  type: 'file_diffs'
  diffs: FileDiffMetadata[]
}

interface FileDiffMetadata {
  type: 'file_diff'
  action: 'create' | 'edit' | 'patch' | 'delete'
  path: string
  oldPath: string | null
  newPath: string | null
  unifiedDiff: string
}

function fileDiffsMetadata(diffs: FileDiffMetadata[]): FileDiffsMetadata {
  return { type: 'file_diffs', diffs }
}

function operationDiffMetadata(operation: PatchOperation): FileDiffMetadata {
  if (operation.type === 'delete') {
    return fileDiffMetadata('delete', operation.oldPath, null, operation.original, '')
  }
  return fileDiffMetadata('patch', operation.oldPath, operation.newPath, operation.original, operation.content)
}

function fileDiffMetadata(
  action: FileDiffMetadata['action'],
  oldPath: string | null,
  newPath: string | null,
  oldText: string,
  newText: string,
): FileDiffMetadata {
  const path = newPath ?? oldPath
  if (!path) throw new Error('Missing diff path')
  return {
    type: 'file_diff',
    action,
    path,
    oldPath,
    newPath,
    unifiedDiff: unifiedDiff(oldPath, newPath, oldText, newText),
  }
}

function unifiedDiff(oldPath: string | null, newPath: string | null, oldText: string, newText: string): string {
  const oldLines = diffLines(oldText)
  const newLines = diffLines(newText)
  const lines = [
    `--- ${oldPath === null ? '/dev/null' : `a/${oldPath}`}`,
    `+++ ${newPath === null ? '/dev/null' : `b/${newPath}`}`,
    `@@ -${diffRange(oldLines.length)} +${diffRange(newLines.length)} @@`,
    ...oldLines.map((line) => `-${line}`),
    ...newLines.map((line) => `+${line}`),
  ]
  return `${lines.join('\n')}\n`
}

function diffLines(text: string): string[] {
  if (text === '') return []
  const body = text.endsWith('\n') ? text.slice(0, -1) : text
  return body === '' ? [] : body.split('\n')
}

function diffRange(lineCount: number): string {
  return lineCount === 0 ? '0,0' : `1,${lineCount}`
}
