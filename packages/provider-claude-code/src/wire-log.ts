import { appendFileSync, mkdirSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import process from 'node:process'
import { z } from 'zod'

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
 * The wire log's environment. Logging is on by default so the raw provider
 * request/response stream is always retained for diagnostics; `0` or `false`
 * turns it off, and any other spelling is a configuration error rather than a
 * silently ignored setting.
 */
const wireLogEnvSchema = z.object({
  DEMI_CLAUDE_WIRE_LOG: z.stringbool({
    truthy: ['1', 'true'],
    falsy: ['0', 'false'],
    case: 'sensitive',
  }).default(true),
  DEMI_CLAUDE_WIRE_LOG_DIR: z.string().min(1).optional(),
})

/**
 * Resolves the directory the claude-code wire log is written to, or null when
 * logging is off. `DEMI_CLAUDE_WIRE_LOG_DIR` relocates it.
 */
export function resolveWireLogDir(): string | null {
  const env = wireLogEnvSchema.parse(process.env)
  if (!env.DEMI_CLAUDE_WIRE_LOG)
    return null
  return env.DEMI_CLAUDE_WIRE_LOG_DIR
    ?? join(tmpdir(), 'demi-claude-wire')
}

export function createClaudeWireLog(sessionId: string): ClaudeWireLog {
  const dir = resolveWireLogDir()
  if (!dir)
    return NULL_WIRE_LOG

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
