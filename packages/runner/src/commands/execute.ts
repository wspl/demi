import type { CommandContext, CommandModule, CommandResult } from '@demicodes/shell'
import { errorMessage, noop } from '@demicodes/utils'
import type { IORequest, IOReplyValue, RunnerMessage, WorkerMessage } from './worker-messages'

declare const DEMI_COMMAND_WORKER_SOURCE: string

/** Each module gets an isolated JS runtime; terminate can interrupt a CPU-bound module. */
export async function workerModule(path: string): Promise<CommandModule> {
  return context => executeInWorker(path, context)
}

function createCommandWorker(): Worker {
  const source = new Blob([DEMI_COMMAND_WORKER_SOURCE], { type: 'text/javascript' })
  const url = URL.createObjectURL(source)
  try {
    return new Worker(url)
  } finally {
    URL.revokeObjectURL(url)
  }
}

type WorkerOutcome =
  | { type: 'exit'; exitCode: number }
  | { type: 'error'; error: unknown }

function executeInWorker(path: string, context: CommandContext): Promise<CommandResult> {
  return new Promise((resolve, reject) => {
    const worker = createCommandWorker()
    const input = context.stdin[Symbol.asyncIterator]()
    let finished = false

    function finish(outcome: WorkerOutcome): void {
      if (finished) return
      finished = true

      context.signal.removeEventListener('abort', abort)
      worker.terminate()
      // Closing stdin must not delay cancellation or replace the command's result.
      void input.return?.().catch(noop)

      if (outcome.type === 'error') {
        reject(outcome.error)
      } else {
        resolve({ exitCode: outcome.exitCode })
      }
    }

    function abort(): void {
      finish({ type: 'error', error: context.signal.reason ?? new Error('command cancelled') })
    }

    function send(message: RunnerMessage): void {
      if (!finished) worker.postMessage(message)
    }

    function replyToRequest(request: IORequest): void {
      void performIO(request, input, context).then(
        value => send({ type: 'reply', id: request.id, value }),
        error => send({ type: 'reply', id: request.id, error: errorMessage(error) }),
      )
    }

    context.signal.addEventListener('abort', abort, { once: true })
    if (context.signal.aborted) {
      abort()
      return
    }

    worker.onerror = () => {
      finish({ type: 'error', error: new Error('command worker failed') })
    }

    worker.onmessage = (event: MessageEvent<WorkerMessage>) => {
      const message = event.data
      switch (message.type) {
        case 'exit': {
          const { exitCode } = message
          if (!Number.isInteger(exitCode) || exitCode < 0 || exitCode > 255) {
            finish({ type: 'error', error: new Error('invalid command exit code') })
          } else {
            finish({ type: 'exit', exitCode })
          }
          return
        }
        case 'error':
          finish({ type: 'error', error: new Error(message.message) })
          return
        default:
          replyToRequest(message)
      }
    }

    send({
      type: 'run',
      path,
      args: context.args,
      cwd: context.cwd,
      env: context.env,
    })
  })
}

async function performIO(
  request: IORequest,
  input: AsyncIterator<Uint8Array>,
  context: CommandContext,
): Promise<IOReplyValue> {
  switch (request.type) {
    case 'input': {
      const next = await input.next()
      return next.done ? null : next.value
    }
    case 'output': {
      const { stream, bytes } = request
      if (!(bytes instanceof Uint8Array) || (stream !== 'stdout' && stream !== 'stderr')) {
        throw new Error('invalid command worker request')
      }
      await context[stream](bytes)
      return null
    }
    default:
      throw new Error('invalid command worker request')
  }
}
