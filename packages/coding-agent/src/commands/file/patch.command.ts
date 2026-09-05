import type { CommandContext, CommandResult, HostFileSystem } from '@demicodes/shell'

/**
 * `demi file patch` with a unified diff on stdin: every file is planned
 * before any is written, and a failed write rolls the earlier ones back.
 */
export default async function patch(ctx: CommandContext<{ patch: string }>): Promise<CommandResult> {
  const files = new Files(ctx.fs, ctx.cwd)
  let patches: FilePatch[]
  let operations: PatchOperation[]
  try {
    patches = parseUnifiedDiff(ctx.args.patch)
    operations = await planPatchOperations(files, patches)
  } catch (error) {
    await ctx.stderr(`${message(error)}\n`)
    return { exitCode: 1 }
  }

  const applied = await applyPatchOperations(files, operations)
  if (applied.exitCode !== 0) {
    await ctx.stderr(applied.stderr)
    return { exitCode: applied.exitCode }
  }
  await ctx.stdout(`Patched ${patches.length} file(s)\n`)
  return { exitCode: 0 }
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

interface StepResult {
  stdout: string
  stderr: string
  exitCode: number
}

/** The filesystem steps of a patch, each reported as an exit code and stderr rather than thrown. */
class Files {
  constructor(
    private readonly fs: HostFileSystem,
    private readonly cwd: string,
  ) {}

  async exists(path: string): Promise<boolean> {
    if (pathValidationError(path)) return false
    return this.fs.exists(path, { cwd: this.cwd })
  }

  async read(path: string): Promise<StepResult> {
    const pathError = pathValidationError(path)
    if (pathError) return { stdout: '', stderr: `${pathError}\n`, exitCode: 1 }
    try {
      return { stdout: new TextDecoder().decode(await this.fs.readFile(path, { cwd: this.cwd })), stderr: '', exitCode: 0 }
    } catch (error) {
      return { stdout: '', stderr: `${message(error)}\n`, exitCode: 1 }
    }
  }

  async write(path: string, content: string): Promise<StepResult> {
    const pathError = pathValidationError(path)
    if (pathError) return { stdout: '', stderr: `${pathError}\n`, exitCode: 1 }
    try {
      await this.fs.writeFile(path, new TextEncoder().encode(content), { cwd: this.cwd, createParents: true })
      return { stdout: '', stderr: '', exitCode: 0 }
    } catch (error) {
      return { stdout: '', stderr: `${message(error)}\n`, exitCode: 1 }
    }
  }

  async delete(path: string): Promise<StepResult> {
    const pathError = pathValidationError(path)
    if (pathError) return { stdout: '', stderr: `${pathError}\n`, exitCode: 1 }
    try {
      await this.fs.rm(path, { cwd: this.cwd, force: true })
      return { stdout: '', stderr: '', exitCode: 0 }
    } catch (error) {
      return { stdout: '', stderr: `${message(error)}\n`, exitCode: 1 }
    }
  }
}

async function applyPatchOperations(files: Files, operations: PatchOperation[]): Promise<StepResult> {
  const rollback: RollbackAction[] = []

  const fail = async (failure: StepResult): Promise<StepResult> => {
    const rollbackFailures: StepResult[] = []
    for (let index = rollback.length - 1; index >= 0; index -= 1) {
      const result = await runRollbackAction(files, rollback[index]!)
      if (result.exitCode !== 0) rollbackFailures.push(result)
    }
    return appendRollbackFailures(failure, rollbackFailures)
  }

  for (const operation of operations) {
    if (operation.type === 'delete') {
      rollback.push({ type: 'write', path: operation.path, content: operation.original })
      const deleted = await files.delete(operation.path)
      if (deleted.exitCode !== 0) return fail(deleted)
      continue
    }

    rollback.push(rollbackForWrite(operation))
    const write = await files.write(operation.path, operation.content)
    if (write.exitCode !== 0) return fail(write)

    if (operation.deletePath) {
      rollback.push({ type: 'write', path: operation.deletePath, content: operation.original })
      const deleted = await files.delete(operation.deletePath)
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

async function runRollbackAction(files: Files, action: RollbackAction): Promise<StepResult> {
  if (action.type === 'delete') return files.delete(action.path)
  return files.write(action.path, action.content)
}

function appendRollbackFailures(failure: StepResult, rollbackFailures: StepResult[]): StepResult {
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
    } else if (line !== '') throw new Error(`Invalid patch line: ${line}`)
  }

  if (patches.length === 0) throw new Error('Invalid patch: no files')
  for (const filePatch of patches) {
    if (!filePatch.oldPath && !filePatch.newPath) throw new Error('Invalid patch: both file paths are /dev/null')
    if (filePatch.hunks.length === 0) throw new Error(`Invalid patch: ${patchDisplayPath(filePatch)} has no hunks`)
  }
  return patches
}

async function planPatchOperations(files: Files, patches: FilePatch[]): Promise<PatchOperation[]> {
  const operations: PatchOperation[] = []

  for (const filePatch of patches) {
    const targetPath = filePatch.newPath ?? filePatch.oldPath
    if (!targetPath) throw new Error('Invalid patch: missing target path')
    assertValidPath(targetPath)

    const isCreate = filePatch.oldPath === null
    const isDelete = filePatch.newPath === null
    let original = ''

    if (isCreate) {
      if (await files.exists(targetPath)) throw new Error(`File already exists: ${targetPath}`)
    } else {
      const oldPath = filePatch.oldPath
      if (oldPath === null) throw new Error('Invalid patch: missing old path')
      assertValidPath(oldPath)
      const read = await files.read(oldPath)
      if (read.exitCode !== 0) throw new Error(read.stderr.trim() || `Failed to read ${oldPath}`)
      original = read.stdout
    }

    if (filePatch.newPath) assertValidPath(filePatch.newPath)
    if (filePatch.oldPath && filePatch.newPath && filePatch.oldPath !== filePatch.newPath && (await files.exists(filePatch.newPath))) {
      throw new Error(`File already exists: ${filePatch.newPath}`)
    }

    const applied = applyFilePatch(original, filePatch)
    if (isDelete) {
      if (applied !== '') throw new Error(`Delete patch leaves content in ${targetPath}`)
      operations.push({ type: 'delete', path: targetPath, original, oldPath: targetPath, newPath: null })
    } else {
      const deletePath = filePatch.oldPath && filePatch.newPath && filePatch.oldPath !== filePatch.newPath ? filePatch.oldPath : undefined
      operations.push({
        type: 'write',
        path: targetPath,
        content: applied,
        original,
        oldPath: filePatch.oldPath,
        newPath: targetPath,
        deletePath,
      })
    }
  }

  return operations
}

function applyFilePatch(content: string, filePatch: FilePatch): string {
  const file = splitTextFile(content)
  const lines = file.lines
  let finalNewline = file.finalNewline
  let offset = 0

  for (const hunk of filePatch.hunks) {
    const index = hunk.oldStart === 0 ? 0 : hunk.oldStart - 1 + offset
    const oldLines = hunk.lines.filter((line) => line.kind !== 'add').map((line) => line.text)
    const newLines = hunk.lines.filter((line) => line.kind !== 'remove').map((line) => line.text)
    const actual = lines.slice(index, index + oldLines.length)
    if (!arraysEqual(actual, oldLines)) {
      throw new Error(`Patch does not apply to ${patchDisplayPath(filePatch)} at line ${hunk.oldStart}`)
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

function stripDiffPathMetadata(rawPath: string): string {
  const trimmed = rawPath.trim()
  const tabIndex = trimmed.indexOf('\t')
  if (tabIndex !== -1) return trimmed.slice(0, tabIndex)
  return trimmed.replace(/\s+\d{4}-\d{2}-\d{2}(?:[ T]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:\s+[+-]\d{4})?)?$/, '')
}

function patchDisplayPath(filePatch: FilePatch): string {
  return filePatch.newPath ?? filePatch.oldPath ?? '<unknown>'
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

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
