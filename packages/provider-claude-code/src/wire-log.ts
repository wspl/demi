import { appendFileSync, mkdirSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import process from 'node:process'

export type WireDirection = 'spawn' | 'in' | 'out' | 'err' | 'exit'

export interface ClaudeWireLog {
  readonly path: string | null
  record(direction: WireDirection, data: unknown): void
}

const NULL_WIRE_LOG: ClaudeWireLog = {
  path: null,
  record() {},
}

/**
 * Resolves the directory the claude-code wire log is written to. Default-on so the raw
 * provider request/response stream is always retained for diagnostics; set
 * `DEMI_CLAUDE_WIRE_LOG=0` to disable, or `DEMI_CLAUDE_WIRE_LOG_DIR` to relocate.
 */
export function resolveWireLogDir(): string | null {
  if (process.env.DEMI_CLAUDE_WIRE_LOG === '0') return null
  return process.env.DEMI_CLAUDE_WIRE_LOG_DIR ?? join(tmpdir(), 'demi-claude-wire')
}

export function createClaudeWireLog(sessionId: string): ClaudeWireLog {
  const dir = resolveWireLogDir()
  if (!dir) return NULL_WIRE_LOG

  try {
    mkdirSync(dir, { recursive: true })
  } catch {
    return NULL_WIRE_LOG
  }

  const safeSession = sessionId.replace(/[^a-zA-Z0-9_-]/g, '_') || 'session'
  const path = join(dir, `claude-${safeSession}.jsonl`)

  return {
    path,
    record(direction, data) {
      const entry = { ts: new Date().toISOString(), dir: direction, data }
      try {
        appendFileSync(path, `${JSON.stringify(entry)}\n`)
      } catch {
        // Diagnostics must never break the run.
      }
    },
  }
}
