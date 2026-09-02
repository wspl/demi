// Byte streams over tinyjs handles, and the process's own standard streams.
import * as fs from 'tinyjs:fs'
import { stderr, stdin, stdout } from 'tinyjs:runtime'
import type { CommandWriter } from '@demicodes/shell'
import { toBytes } from '@demicodes/utils'

const READ_CHUNK = 64 * 1024

/**
 * Pulls a handle to its end. `close` releases the handle when the stream
 * ends or the consumer stops early; the standard streams are never closed.
 */
export async function* readHandle(fd: number, close: boolean): AsyncIterable<Uint8Array> {
  try {
    for (;;) {
      const chunk = await fs.read(fd, READ_CHUNK)
      if (chunk === null) return
      yield chunk
    }
  } finally {
    if (close) fs.close(fd)
  }
}

export function writerFor(fd: number): CommandWriter {
  return (data) => fs.write(fd, toBytes(data))
}

/** The process's stdin as a byte stream. */
export function stdinStream(): AsyncIterable<Uint8Array> {
  return readHandle(stdin, false)
}

export const stdoutWriter = (): CommandWriter => writerFor(stdout)
export const stderrWriter = (): CommandWriter => writerFor(stderr)
