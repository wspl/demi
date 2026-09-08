import { createRunnerFileSystem } from '../machine/fs'
import { importCommandModule } from '@demicodes/shell'
import type { CommandResult } from '@demicodes/shell'
import { errorMessage, encodeUtf8 } from '@demicodes/utils'

const scope = globalThis as unknown as { onmessage: ((event: MessageEvent) => void) | null; postMessage(value: unknown): void }
let sequence = 0
const pending = new Map<number, { resolve(value: unknown): void; reject(error: Error): void }>()
function request(type: string, data: Record<string, unknown> = {}): Promise<unknown> {
  const id = ++sequence
  return new Promise((resolve, reject) => { pending.set(id, { resolve, reject }); scope.postMessage({ type, id, ...data }) })
}
let started = false
scope.onmessage = event => {
  const message = event.data
  if (message.type === 'reply') {
    const waiter = pending.get(message.id)
    pending.delete(message.id)
    if (message.error) waiter?.reject(new Error(message.error))
    else waiter?.resolve(message.value)
    return
  }
  if (message.type !== 'run' || started) throw new Error('unexpected command worker message')
  started = true
  const { path, args, cwd, env } = message
  const stdin: AsyncIterable<Uint8Array> = {
    async *[Symbol.asyncIterator]() {
      for (;;) {
        const bytes = await request('input') as Uint8Array | null
        if (bytes === null) return
        yield bytes
      }
    },
  }
  const write = (stream: string) => async (data: string | Uint8Array) => {
    await request('output', { stream, bytes: typeof data === 'string' ? encodeUtf8(data) : data })
  }
  void (async () => {
    const command = await importCommandModule(path)
    const result: CommandResult = await command({ args, cwd, env, fs: createRunnerFileSystem(cwd), stdin, stdout: write('stdout'), stderr: write('stderr'), signal: new AbortController().signal })
    scope.postMessage({ type: 'exit', exitCode: result.exitCode })
  })().catch(error => scope.postMessage({ type: 'error', message: errorMessage(error) }))
}
