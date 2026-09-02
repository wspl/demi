import { decodeUtf8Strict, tail, utf8Bytes, utf8Slice } from '@demicodes/utils'
import type {
  BinaryStdout,
  ShellCommandStatus,
  ShellOutputChunk,
  ShellOutputRecordChunk,
  ShellOutputView,
  ShellStreamView,
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
  /** The target directory holding the output files (the runner's tee); absent for hostless. */
  outputDir?: string
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
  binaryStdout?: BinaryStdout
  /**
   * The length of each stream on the target when the record holds only its
   * view (a runner's head and tail); absent when the text is the stream.
   */
  stdoutBytes?: number
  stderrBytes?: number
  /** Output limit of the exec that started this command (binary carry cap). */
  outputLimitBytes: number
}

export function createCommandRecord(fields: {
  id: string
  shellId: string
  commandStorageId: string
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
  }
}

/**
 * The status the tools see: each stream's delta since the last view (the
 * record keeps the cursor), capped at `maxOutputBytes`, plus its tail.
 */
export function commandStatusView(record: ShellCommandRecord, maxOutputBytes: number): ShellCommandStatus {
  const stdout = streamView(record, 'stdout', maxOutputBytes)
  const stderr = streamView(record, 'stderr', maxOutputBytes)
  const output = mergedOutputView(record, maxOutputBytes)
  const base = {
    shellId: record.shellId,
    commandId: record.id,
    ...(record.outputDir !== undefined ? { outputDir: record.outputDir } : {}),
    stdout,
    stderr,
    output,
    runningMs: Date.now() - record.startedAt,
    idleMs: Date.now() - record.lastOutputAt,
  }
  if (record.status === 'exited') {
    const result: ShellCommandStatus = {
      ...base,
      status: 'exited',
      exitCode: record.exitCode ?? 0,
    }
    if (record.binaryStdout) result.binaryStdout = record.binaryStdout
    return result
  }
  if (record.status === 'aborted') return { ...base, status: 'aborted' }
  return { ...base, status: 'running' }
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
  /** Where the raw stream lives on the target; absent when it was not kept. */
  rawPath?: string,
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
  }${rawPath ? `; raw bytes at ${rawPath}` : '; not kept beyond this view'}>\n`
  return { text, binary }
}

/** Marks the record exited with the given streams, replacing any streamed view of a binary stream. */
export function settleExited(record: ShellCommandRecord, exitCode: number, stdout: string, stderr: string, binary: BinaryStdout | undefined): void {
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
}

function streamView(record: ShellCommandRecord, stream: 'stdout' | 'stderr', maxOutputBytes: number): ShellStreamView {
  const text = stream === 'stdout' ? record.stdout : record.stderr
  const totalBytes = utf8Bytes(text)
  const offset = stream === 'stdout' ? record.stdoutOffset : record.stderrOffset
  const boundedOffset = clampOffset(offset, totalBytes)
  const available = Math.max(0, totalBytes - boundedOffset)
  const byteLimit = Math.max(0, Math.floor(maxOutputBytes))
  const takeBytes = byteLimit === 0 ? available : Math.min(available, byteLimit)
  const delta = utf8Slice(text, boundedOffset, boundedOffset + takeBytes)
  const nextOffset = boundedOffset + utf8Bytes(delta)
  const truncated = nextOffset < totalBytes
  if (stream === 'stdout') record.stdoutOffset = nextOffset
  else record.stderrOffset = nextOffset
  return {
    ...(record.outputDir !== undefined ? { path: `${record.outputDir}/${stream}.txt` } : {}),
    offset: nextOffset,
    delta,
    tail: tailString(text),
    bytes: (stream === 'stdout' ? record.stdoutBytes : record.stderrBytes) ?? totalBytes,
    truncated,
  }
}

function mergedOutputView(record: ShellCommandRecord, maxOutputBytes: number): ShellOutputView {
  const totalBytes = record.outputChunks.reduce((total, chunk) => total + chunk.bytes, 0)
  const offset = clampOffset(record.outputOffset, totalBytes)
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
  record.outputOffset = nextOffset
  return {
    ...(record.outputDir !== undefined ? { path: record.outputDir } : {}),
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
