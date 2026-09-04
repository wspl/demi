// The command-mode end of the local relay: the manifest on a cache miss,
// and the `RpcTransport` the loader hands `rpc` invocations to.
import { connectUnix, msgpackDecode, msgpackEncode, type StreamSocket } from '../machine'
import type { RpcTransport } from '@demicodes/command-loader'
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

/**
 * Forwards one `rpc` invocation over the relay: the request, then the pipe
 * as `pipe` frames and the live stdin as it arrives, while stdout and
 * stderr stream back into the invocation's writers until the exit. The
 * runner carries the pipe on as an HTTP stream (`runner.md` § Pipes); this
 * process never sees where.
 */
export function relayRpc(socketPath: string, ids: { agentSessionId: string; shellId: string }): RpcTransport {
  return async (invocation) => {
    const socket = await connectUnix(socketPath)
    const write = serialWriter(socket)
    const abort = () => socket.close()
    invocation.signal.addEventListener('abort', abort, { once: true })
    try {
      await write({
        type: 'rpc',
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
      for await (const reply of framesOf(socket.input, codec, relayReplySchema)) {
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
    }
  }
}

/** Frames written one after another: a stream socket takes one write at a time. */
function serialWriter(socket: StreamSocket): (request: RelayRequest) => Promise<void> {
  let chain = Promise.resolve()
  return (request) => {
    const next = chain.then(() => socket.write(frameOf(codec, request)))
    chain = next.catch(() => {})
    return next
  }
}
