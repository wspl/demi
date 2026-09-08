import type { CommandWriter } from '@demicodes/shell'
import { toBytes } from '@demicodes/utils'

/** Standard writers are shared so successive commands do not relock stdout. */
let out: WritableStreamDefaultWriter<Uint8Array> | undefined
let err: WritableStreamDefaultWriter<Uint8Array> | undefined
export const stdoutWriter = (): CommandWriter => {
  out ??= tjs.stdout.getWriter()
  return (data) => out!.write(toBytes(data))
}
export const stderrWriter = (): CommandWriter => {
  err ??= tjs.stderr.getWriter()
  return (data) => err!.write(toBytes(data))
}
export const stdinStream = (): AsyncIterable<Uint8Array> => tjs.stdin
