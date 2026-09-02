import type { TinybashFs } from '../host'
import type { Redirect, Word } from '../grammar/ast'
import { type ExpansionScope, expandHeredoc, expandSingle, expandToFields, fieldText, wordSource } from '../grammar/expand'
import { expandGlob, hasGlobChars } from '../grammar/glob'
import { resolvePath } from '../outside/namespace'
import { type Writer } from './stream'
import { strerror } from '../builtins/errors'
import { bytesStream, concatBytes, emptyByteStream, encodeUtf8, toBytes } from '@demicodes/utils'

export interface Channels {
  stdin: AsyncIterable<Uint8Array>
  stdout: Writer
  stderr: Writer
}

/** Output bound for a file: collected while the command runs, appended when it ends. */
class FileSink {
  private readonly chunks: Uint8Array[] = []
  constructor(private readonly fs: TinybashFs, private readonly path: string) {}
  readonly write: Writer = (data) => {
    const bytes = toBytes(data)
    if (bytes.length > 0) this.chunks.push(bytes)
  }
  async flush(): Promise<void> {
    if (this.chunks.length === 0) return
    await this.fs.appendFile(this.path, concatBytes(this.chunks))
    this.chunks.length = 0
  }
}

export class RedirectError extends Error {
  constructor(readonly path: string, readonly detail: string) {
    super(`${path}: ${detail}`)
  }
}

/**
 * The one path a redirection names: the word after the operator is expanded,
 * split and globbed like any other, and bash refuses it when that yields
 * anything but exactly one word.
 */
async function redirectTarget(word: Word, scope: ExpansionScope, fs: TinybashFs): Promise<string> {
  const fields = expandToFields(word, scope)
  if (fields.length !== 1) throw new RedirectError(wordSource(word), 'ambiguous redirect')
  const field = fields[0]!
  if (!hasGlobChars(field)) return fieldText(field)
  const matches = await expandGlob(field, scope.cwd, fs)
  if (matches.length !== 1) throw new RedirectError(wordSource(word), 'ambiguous redirect')
  return matches[0]!
}

/**
 * Applies a command's redirections in order over the channels it inherits.
 * Files are created or truncated now, as the shell would before running the
 * command; `flush` writes what the command produced.
 */
export async function applyRedirects(redirects: readonly Redirect[], inherited: Channels, scope: ExpansionScope, fs: TinybashFs): Promise<{ channels: Channels; flush: () => Promise<void> }> {
  const channels: Channels = { ...inherited }
  const sinks: FileSink[] = []
  for (const redirect of redirects) {
    switch (redirect.kind) {
      case 'file': {
        const target = await redirectTarget(redirect.path, scope, fs)
        const resolved = resolvePath(scope.cwd, target)
        let writer: Writer
        if (resolved === '/dev/null') {
          writer = () => {}
        } else {
          try {
            // Open now, as the shell does: a directory or a missing parent fails here, not after the command ran.
            if (redirect.mode === 'truncate') await fs.writeFile(resolved, new Uint8Array(0))
            else await fs.appendFile(resolved, new Uint8Array(0))
          } catch (error) {
            throw new RedirectError(target, strerror(error))
          }
          const sink = new FileSink(fs, resolved)
          sinks.push(sink)
          writer = sink.write
        }
        if (redirect.target === 'stdout' || redirect.target === 'both') channels.stdout = writer
        if (redirect.target === 'stderr' || redirect.target === 'both') channels.stderr = writer
        break
      }
      case 'input': {
        const target = await redirectTarget(redirect.path, scope, fs)
        const resolved = resolvePath(scope.cwd, target)
        if (resolved === '/dev/null') {
          channels.stdin = emptyByteStream()
          break
        }
        try {
          const stat = await fs.stat(resolved)
          if (stat.isDirectory) {
            // bash opens the directory fine; the first read fails inside the command with EISDIR.
            channels.stdin = directoryStream(target)
            break
          }
          channels.stdin = bytesStream(await fs.readFile(resolved))
        } catch (error) {
          throw new RedirectError(target, strerror(error))
        }
        break
      }
      case 'stderr-to-stdout':
        channels.stderr = channels.stdout
        break
      case 'heredoc':
        channels.stdin = bytesStream(encodeUtf8(expandHeredoc(redirect.body, scope)))
        break
      case 'herestring':
        channels.stdin = bytesStream(encodeUtf8(`${expandSingle(redirect.word, scope)}\n`))
        break
    }
  }
  return {
    channels,
    flush: async () => {
      for (const sink of sinks) await sink.flush()
    },
  }
}

async function* directoryStream(path: string): AsyncIterable<Uint8Array> {
  const error = new Error(`Is a directory`) as Error & { code: string; path: string }
  error.code = 'EISDIR'
  error.path = path
  throw error
}
