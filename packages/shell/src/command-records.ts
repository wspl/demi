import { decodeUtf8Strict, tail, utf8Bytes, utf8Slice } from '@demicodes/utils'
import type { CommandArtifactStore } from './command-artifact-store'
import type {
  BashAuditEvent,
  BinaryStdout,
  ShellCommandStatus,
  ShellOutputChunk,
  ShellOutputRecordChunk,
  ShellOutputView,
  ShellStreamView,
  ShellViewInput,
} from './shell-environment'

/**
 * One command's record — what `shell_status` reads — and the views cut from
 * it. Shared by every shell environment: the record does not know what ran
 * the script.
 */
export interface ShellCommandRecord {
  id: string
  shellId: string
  commandStorageId: string
  /** Real directory the artifact files (stdout.txt/stderr.txt/meta.json/stdout.bin) live in. */
  artifactDir: string
  script: string
  startedAt: number
  lastOutputAt: number
  status: 'running' | 'exited' | 'aborted'
  stdout: string
  stderr: string
  stdoutOffset: number
  stderrOffset: number
  outputChunks: ShellOutputRecordChunk[]
  outputOffset: number
  exitCode?: number
  audit: BashAuditEvent[]
  binaryStdout?: BinaryStdout
  /** Full binary stream bytes awaiting their one-time write to stdout.bin. */
  pendingBinaryArtifact?: Uint8Array
  /** Output limit of the exec that started this command (binary carry cap). */
  outputLimitBytes: number
  persistedFingerprint?: string
}

export function createCommandRecord(fields: {
  id: string
  shellId: string
  commandStorageId: string
  artifactDir: string
  script: string
  outputLimitBytes: number
}): ShellCommandRecord {
  const now = Date.now()
  return {
    ...fields,
    startedAt: now,
    lastOutputAt: now,
    status: 'running',
    stdout: '',
    stderr: '',
    stdoutOffset: 0,
    stderrOffset: 0,
    outputChunks: [],
    outputOffset: 0,
    audit: [],
  }
}

/**
 * The status the tools see: stream views cut at the caller's offsets, then
 * the record persisted through `artifacts` — absent for an environment whose
 * output files are written where the command ran (the runner's tee).
 */
export function commandStatusView(
  record: ShellCommandRecord,
  input: ShellViewInput,
  defaultOutputLimitBytes: number,
  artifacts?: CommandArtifactStore,
): ShellCommandStatus {
  const maxOutputBytes = input.maxOutputBytes ?? defaultOutputLimitBytes
  const stdout = streamView(record, 'stdout', input.stdoutOffset, maxOutputBytes)
  const stderr = streamView(record, 'stderr', input.stderrOffset, maxOutputBytes)
  const output = mergedOutputView(record, input.outputOffset, maxOutputBytes)
  const base = {
    shellId: record.shellId,
    commandId: record.id,
    artifactDir: record.artifactDir,
    stdout,
    stderr,
    output,
    runningMs: Date.now() - record.startedAt,
    idleMs: Date.now() - record.lastOutputAt,
  }
  if (artifacts) persistCommandArtifact(artifacts, record)
  if (record.status === 'exited') {
    const result: ShellCommandStatus = {
      ...base,
      status: 'exited',
      exitCode: record.exitCode ?? 0,
      audit: record.audit,
    }
    if (record.binaryStdout) result.binaryStdout = record.binaryStdout
    return result
  }
  if (record.status === 'aborted') return { ...base, status: 'aborted' }
  return { ...base, status: 'running' }
}

export function persistCommandArtifact(artifacts: CommandArtifactStore, record: ShellCommandRecord): void {
  const fingerprint = `${record.status}:${record.exitCode ?? ''}:${record.stdout.length}:${record.stderr.length}:${record.binaryStdout?.totalBytes ?? ''}`
  if (record.persistedFingerprint === fingerprint) return
  record.persistedFingerprint = fingerprint
  const stdoutBin = record.pendingBinaryArtifact
  record.pendingBinaryArtifact = undefined
  artifacts.persist(record.commandStorageId, record.id, {
    meta: `${JSON.stringify(commandArtifactMeta(record), null, 2)}\n`,
    stdout: record.stdout,
    stderr: record.stderr,
    ...(stdoutBin ? { stdoutBin } : {}),
  })
}

/**
 * The final-stream boundary: valid UTF-8 becomes text; anything else stays
 * raw bytes on the record (`binaryStdout`) with a placeholder in the text
 * channel — raw binary never enters the text render. Raw bytes answer to
 * their own ceiling, not the text budget: this stream exists to be looked
 * at, and the text cap is sized to stop a log flood.
 */
export function finalStdoutBoundary(
  bytes: Uint8Array,
  binaryLimitBytes: number,
  artifactDir: string,
): { text: string; binary?: BinaryStdout } {
  const strict = decodeUtf8Strict(bytes)
  if (strict !== null) return { text: strict }
  const truncated = bytes.length > binaryLimitBytes
  const binary: BinaryStdout = {
    data: truncated ? bytes.slice(0, binaryLimitBytes) : bytes,
    truncated,
    totalBytes: bytes.length,
    limitBytes: binaryLimitBytes,
  }
  const text = `<binary stdout: ${bytes.length} bytes${
    truncated ? `, exceeds the ${binaryLimitBytes}-byte binary limit` : ''
  }; raw bytes at ${artifactDir}/stdout.bin>\n`
  return { text, binary }
}

/** Marks the record exited with the given streams, replacing any streamed view of a binary stream. */
export function settleExited(record: ShellCommandRecord, exitCode: number, stdout: string, stderr: string, binary: BinaryStdout | undefined, audit: BashAuditEvent[]): void {
  record.stdout = stdout
  record.stderr = stderr
  if (binary) {
    record.binaryStdout = binary
    record.outputChunks = []
  }
  if (record.outputChunks.length === 0) {
    appendRecordOutput(record, 'stdout', stdout)
    appendRecordOutput(record, 'stderr', stderr)
  }
  ensureRecordOutputCoverage(record)
  record.lastOutputAt = Date.now()
  record.status = 'exited'
  record.exitCode = exitCode
  record.audit = [...audit]
}

function streamView(
  record: ShellCommandRecord,
  stream: 'stdout' | 'stderr',
  explicitOffset: number | undefined,
  maxOutputBytes: number,
): ShellStreamView {
  const text = stream === 'stdout' ? record.stdout : record.stderr
  const totalBytes = utf8Bytes(text)
  const offset = explicitOffset ?? (stream === 'stdout' ? record.stdoutOffset : record.stderrOffset)
  const boundedOffset = clampOffset(offset, totalBytes)
  const available = Math.max(0, totalBytes - boundedOffset)
  const byteLimit = Math.max(0, Math.floor(maxOutputBytes))
  const takeBytes = byteLimit === 0 ? available : Math.min(available, byteLimit)
  const delta = utf8Slice(text, boundedOffset, boundedOffset + takeBytes)
  const nextOffset = boundedOffset + utf8Bytes(delta)
  const truncated = nextOffset < totalBytes
  if (explicitOffset === undefined) {
    if (stream === 'stdout') record.stdoutOffset = nextOffset
    else record.stderrOffset = nextOffset
  }
  return {
    path: `${record.artifactDir}/${stream}.txt`,
    offset: nextOffset,
    delta,
    tail: tailString(text),
    bytes: totalBytes,
    truncated,
  }
}

function mergedOutputView(
  record: ShellCommandRecord,
  explicitOffset: number | undefined,
  maxOutputBytes: number,
): ShellOutputView {
  const totalBytes = record.outputChunks.reduce((total, chunk) => total + chunk.bytes, 0)
  const offset = clampOffset(explicitOffset ?? record.outputOffset, totalBytes)
  const byteLimit = Math.max(0, Math.floor(maxOutputBytes))
  const available = Math.max(0, totalBytes - offset)
  let remaining = byteLimit === 0 ? available : Math.min(available, byteLimit)
  const chunks: ShellOutputChunk[] = []

  for (const chunk of record.outputChunks) {
    if (remaining <= 0) break
    const chunkStart = chunk.offset
    const chunkEnd = chunk.offset + chunk.bytes
    if (chunkEnd <= offset) continue
    const start = Math.max(0, offset - chunkStart)
    const take = Math.min(chunk.bytes - start, remaining)
    const text = utf8Slice(chunk.text, start, start + take)
    if (text.length > 0) {
      chunks.push({ stream: chunk.stream, text })
      remaining -= utf8Bytes(text)
    } else {
      remaining -= take
    }
  }

  const text = chunks.map((chunk) => chunk.text).join('')
  const nextOffset = offset + utf8Bytes(text)
  const truncated = nextOffset < totalBytes
  if (explicitOffset === undefined) record.outputOffset = nextOffset
  return {
    path: record.artifactDir,
    offset: nextOffset,
    text,
    tail: tailOutputText(record.outputChunks),
    chunks,
    bytes: totalBytes,
    truncated,
  }
}

export function appendRecordOutput(record: ShellCommandRecord, stream: 'stdout' | 'stderr', text: string): void {
  if (text.length === 0) return
  const offset = record.outputChunks.reduce((total, chunk) => total + chunk.bytes, 0)
  record.outputChunks.push({ stream, text, offset, bytes: utf8Bytes(text) })
}

export function ensureRecordOutputCoverage(record: ShellCommandRecord): void {
  const stdoutBytes = record.outputChunks
    .filter((chunk) => chunk.stream === 'stdout')
    .reduce((total, chunk) => total + chunk.bytes, 0)
  const stderrBytes = record.outputChunks
    .filter((chunk) => chunk.stream === 'stderr')
    .reduce((total, chunk) => total + chunk.bytes, 0)
  if (stdoutBytes === utf8Bytes(record.stdout) && stderrBytes === utf8Bytes(record.stderr)) return
  record.outputChunks = []
  appendRecordOutput(record, 'stdout', record.stdout)
  appendRecordOutput(record, 'stderr', record.stderr)
}

function commandArtifactMeta(record: ShellCommandRecord): Record<string, unknown> {
  return {
    status: record.status,
    shellId: record.shellId,
    commandId: record.id,
    script: record.script,
    startedAt: record.startedAt,
    lastOutputAt: record.lastOutputAt,
    exitCode: record.exitCode ?? null,
    stdout: { path: `${record.artifactDir}/stdout.txt`, bytes: utf8Bytes(record.stdout) },
    stderr: { path: `${record.artifactDir}/stderr.txt`, bytes: utf8Bytes(record.stderr) },
    ...(record.binaryStdout
      ? {
          stdoutBinary: {
            path: `${record.artifactDir}/stdout.bin`,
            bytes: record.binaryStdout.totalBytes,
          },
        }
      : {}),
  }
}

function tailOutputText(chunks: readonly ShellOutputRecordChunk[]): string {
  const maxChars = 4096
  let text = ''
  for (let i = chunks.length - 1; i >= 0 && text.length < maxChars; i -= 1) {
    text = `${chunks[i]!.text}${text}`
  }
  return tailString(text)
}

function clampOffset(value: number, max: number): number {
  if (!Number.isFinite(value) || value <= 0) return 0
  return Math.min(Math.floor(value), max)
}

function tailString(value: string): string {
  return tail(value, 4096)
}
