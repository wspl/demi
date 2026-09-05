// The command-mode end of the local relay: the manifest on a cache miss,
// and the `RpcTransport` the loader hands `rpc` invocations to.
import { connectUnix, msgpackDecode, msgpackEncode, type StreamSocket } from '../machine'
import type { RpcTransport } from '@demicodes/command-loader'
import type { DispatchIO } from '@demicodes/shell'
import { createId, SerialQueue, throwIfAborted } from '@demicodes/utils'
import { frameOf, framesOf, relayReplySchema, type RelayRequest } from './protocol'

const codec = { encode: msgpackEncode, decode: msgpackDecode }

export async function fetchManifest(socketPath: string): Promise<unknown | null> {
  const socket = await connectUnix(socketPath)
  try {
    await socket.write(frameOf(codec, { type: 'manifest' } satisfies RelayRequest))
    for await (const reply of framesOf(socket.input, codec, relayReplySchema)) {
      if (reply.type === 'manifest') return reply.manifest
      if (reply.type === 'error') throw new Error(reply.message)
    }
    throw new Error('relay closed without a manifest')
  } finally {
    socket.close()
  }
}

/** The hint uses its own lifetime connection, so exit or process death clears it without output traffic. */
export function relayRunningHint(socketPath: string, jobId: string, signal: AbortSignal): NonNullable<DispatchIO['onRunningHint']> {
  let socket: StreamSocket | null = null
  const close = () => {
    socket?.close()
    socket = null
    signal.removeEventListener('abort', close)
  }
  return async (hint) => {
    close()
    if (hint === undefined) return
    throwIfAborted(signal)
    signal.addEventListener('abort', close, { once: true })
    try {
      const opened = await connectUnix(socketPath)
      socket = opened
      throwIfAborted(signal)
      await opened.write(frameOf(codec, { type: 'running_hint', jobId, hint } satisfies RelayRequest))
      const reply = await framesOf(opened.input, codec, relayReplySchema)[Symbol.asyncIterator]().next()
      throwIfAborted(signal)
      if (reply.done || reply.value.type !== 'ready') throw new Error('relay did not accept the running hint')
    } catch (error) {
      close()
      throwIfAborted(signal)
      throw error
    }
  }
}

/**
 * Forwards one `rpc` invocation over the relay: the request, then the pipe
 * as `pipe` frames and the live stdin as it arrives, while stdout and
 * stderr stream back into the invocation's writers until the exit. The
 * runner carries the pipe on as an HTTP stream (`runner.md` § Pipes); this
 * process never sees where.
 */
export function relayRpc(socketPath: string, ids: { agentSessionId: string; shellId: string; jobId?: string }): RpcTransport {
  return async (invocation) => {
    throwIfAborted(invocation.signal)
    const socket = await connectUnix(socketPath)
    const callId = createId()
    const writes = new SerialQueue()
    const write = (request: RelayRequest) => writes.run(() => socket.write(frameOf(codec, request)))
    let control: StreamSocket | null = null
    const abort = () => {
      control?.close()
      socket.close()
    }
    const replies = framesOf(socket.input, codec, relayReplySchema)[Symbol.asyncIterator]()
    invocation.signal.addEventListener('abort', abort, { once: true })
    try {
      throwIfAborted(invocation.signal)
      await write({
        type: 'rpc',
        callId,
        ...ids,
        root: invocation.root,
        path: invocation.path,
        argv: invocation.argv,
        args: invocation.args,
        json: invocation.json,
        cwd: invocation.cwd,
        env: invocation.env,
        stdin: invocation.stdin !== null,
      })
      const ready = await replies.next()
      if (ready.done || ready.value.type !== 'ready') throw new Error('relay did not accept the invocation')
      control = await connectUnix(socketPath)
      if (invocation.signal.aborted) abort()
      throwIfAborted(invocation.signal)
      await control.write(frameOf(codec, { type: 'watch', callId } satisfies RelayRequest))
      const watching = await framesOf(control.input, codec, relayReplySchema)[Symbol.asyncIterator]().next()
      if (watching.done || watching.value.type !== 'ready') throw new Error('relay did not accept the lifetime connection')
      // The pipe is finite and forwarded to its end; the live stdin may never
      // end (it is the job's own) and is abandoned with the socket at exit.
      const pipe = invocation.stdin
      if (pipe) {
        void (async () => {
          try {
            for await (const bytes of pipe) {
              if (bytes.byteLength > 0) await write({ type: 'pipe', bytes })
            }
            await write({ type: 'pipe_end' })
          } catch {
            // The socket closed with the exit; nothing more to send.
          }
        })()
      }
      void (async () => {
        try {
          for await (const bytes of invocation.stdinStream) {
            if (bytes.byteLength > 0) await write({ type: 'stdin', bytes })
          }
          await write({ type: 'stdin_end' })
        } catch {
          // The socket closed with the exit; nothing more to send.
        }
      })()
      let exitCode: number | null = null
      for (;;) {
        const next = await replies.next()
        if (next.done) break
        const reply = next.value
        if (reply.type === 'output') await (reply.stream === 'stdout' ? invocation.io.stdout : invocation.io.stderr)(reply.bytes)
        else if (reply.type === 'exit') {
          exitCode = reply.exitCode
          break
        } else if (reply.type === 'error') throw new Error(reply.message)
      }
      socket.close()
      if (exitCode === null) throw new Error('relay closed before the command exited')
      return { exitCode }
    } finally {
      invocation.signal.removeEventListener('abort', abort)
      socket.close()
      control?.close()
    }
  }
}
