import { importCommandModule } from '@demicodes/shell'
import type { CommandWriter } from '@demicodes/shell'
import {
  deferred,
  errorMessage,
  encodeUtf8,
  type Deferred
} from '@demicodes/utils'
import { createRunnerFileSystem } from '../machine/fs'
import type {
  InputRequest,
  IOReplyValue,
  OutputRequest,
  RunCommand,
  RunnerMessage,
  WorkerMessage,
} from './worker-messages'

// txiki exposes these globals in workers; the runner's main-thread types do not.
const scope = globalThis as unknown as {
  onmessage: ((event: MessageEvent<RunnerMessage>) => void) | null
  postMessage(message: WorkerMessage): void
}

let nextRequestId = 0
let started = false
const pendingRequests = new Map<number, Deferred<IOReplyValue>>()

function requestIO(
  request: Omit<InputRequest, 'id'> | Omit<OutputRequest, 'id'>,
): Promise<IOReplyValue> {
  const id = ++nextRequestId
  const reply = deferred<IOReplyValue>()
  pendingRequests.set(id, reply)
  scope.postMessage({ ...request, id })
  return reply.promise
}

const stdin: AsyncIterable<Uint8Array> = {
  async *[Symbol.asyncIterator]() {
    for (;;) {
      const bytes = await requestIO({ type: 'input' })
      if (bytes === null) {
        return
      }
      yield bytes
    }
  },
}

function outputWriter(stream: 'stdout' | 'stderr'): CommandWriter {
  return async data => {
    const bytes = typeof data === 'string' ? encodeUtf8(data) : data
    await requestIO({ type: 'output', stream, bytes })
  }
}

async function runCommand(message: RunCommand): Promise<void> {
  const command = await importCommandModule(message.path)
  const result = await command({
    args: message.args,
    cwd: message.cwd,
    env: message.env,
    fs: createRunnerFileSystem(message.cwd),
    stdin,
    stdout: outputWriter('stdout'),
    stderr: outputWriter('stderr'),
    // Cancellation terminates this worker, including CPU-bound command code.
    signal: new AbortController().signal,
  })
  scope.postMessage({ type: 'exit', exitCode: result.exitCode })
}

scope.onmessage = event => {
  const message = event.data
  if (message.type === 'reply') {
    const pending = pendingRequests.get(message.id)
    pendingRequests.delete(message.id)
    if ('error' in message) {
      pending?.reject(new Error(message.error))
    } else {
      pending?.resolve(message.value)
    }
    return
  }

  if (message.type !== 'run' || started) {
    throw new Error('unexpected command worker message')
  }
  started = true
  void runCommand(message).catch(error => {
    scope.postMessage({ type: 'error', message: errorMessage(error) })
  })
}
