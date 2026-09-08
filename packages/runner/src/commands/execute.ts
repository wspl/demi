import type { CommandModule } from '@demicodes/shell'
import { errorMessage, noop } from '@demicodes/utils'

declare const DEMI_COMMAND_WORKER_SOURCE: string

/** Each module gets an isolated JS runtime; terminate can interrupt a CPU-bound module. */
export function workerModule(path: string): Promise<CommandModule> {
  return Promise.resolve(ctx => new Promise((resolve, reject) => {
    const url = URL.createObjectURL(new Blob([DEMI_COMMAND_WORKER_SOURCE], { type: 'text/javascript' }))
    const worker = new Worker(url)
    URL.revokeObjectURL(url)
    const input = ctx.stdin[Symbol.asyncIterator]()
    let finished = false
    const finish = (error?: unknown, exitCode?: number) => {
      if (finished) return
      finished = true
      ctx.signal.removeEventListener('abort', abort)
      worker.terminate()
      void input.return?.().catch(noop)
      if (error) reject(error)
      else resolve({ exitCode: exitCode! })
    }
    const abort = () => finish(ctx.signal.reason ?? new Error('command cancelled'))
    ctx.signal.addEventListener('abort', abort, { once: true })
    if (ctx.signal.aborted) { abort(); return }
    worker.onerror = () => finish(new Error('command worker failed'))
    worker.onmessage = event => {
      const message = event.data
      if (message.type === 'exit') {
        if (!Number.isInteger(message.exitCode) || message.exitCode < 0 || message.exitCode > 255) finish(new Error('invalid command exit code'))
        else finish(undefined, message.exitCode)
      } else if (message.type === 'error') finish(new Error(message.message))
      else {
        void (async () => {
          if (message.type === 'input') {
            const next = await input.next()
            return next.done ? null : next.value
          }
          if (message.type === 'output' && message.bytes instanceof Uint8Array && (message.stream === 'stdout' || message.stream === 'stderr')) {
            await ctx[message.stream as 'stdout' | 'stderr'](message.bytes)
            return null
          }
          throw new Error('invalid command worker request')
        })().then(value => { if (!finished) worker.postMessage({ type: 'reply', id: message.id, value }) }, error => {
          if (!finished) worker.postMessage({ type: 'reply', id: message.id, error: errorMessage(error) })
        })
      }
    }
    worker.postMessage({ type: 'run', path, args: ctx.args, cwd: ctx.cwd, env: ctx.env })
  }))
}
