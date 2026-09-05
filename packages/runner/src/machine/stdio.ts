// Byte streams over tinyjs handles, and the process's own standard streams.
import * as fs from 'tinyjs:fs'
import { stderr, stdin, stdout } from 'tinyjs:runtime'
import type { CommandWriter } from '@demicodes/shell'
import { toBytes } from '@demicodes/utils'

const READ_CHUNK = 64 * 1024

/**
 * Pulls a handle to its end. `close` releases the handle when the stream
 * ends, fails, or the consumer returns early — before the first read too,
 * so a stream nobody ever pulled can still be let go; the standard streams
 * are never closed.
 */
export function readHandle(fd: number, close: boolean): AsyncIterable<Uint8Array> {
  let ended = false
  const end = (): void => {
    if (ended) return
    ended = true
    if (close) fs.close(fd)
  }
  const done = { done: true as const, value: undefined }
  return {
    [Symbol.asyncIterator]: () => ({
      next: async () => {
        if (ended) return done
        let chunk: Uint8Array | null
        try {
          chunk = await fs.read(fd, READ_CHUNK)
        } catch (error) {
          end()
          throw error
        }
        if (chunk === null) {
          end()
          return done
        }
        return { done: false, value: chunk }
      },
      return: async () => {
        end()
        return done
      },
    }),
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

/** A bounded in-memory pipe: what is written to `write` is read from `read`, with backpressure. */
export function openPipe(): { read: number; write: number } {
  return fs.pipe()
}
