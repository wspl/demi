// The command-mode end of the local relay: the manifest on a cache miss,
// and the `RpcTransport` the loader hands `rpc` invocations to.
import { connectUnix, msgpackDecode, msgpackEncode } from '@demicodes/host-runner'
import type { RpcTransport } from '@demicodes/command-loader'
import { frameOf, framesOf, relayReplySchema, type RelayRequest } from './relay-protocol'

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
 * Forwards one `rpc` invocation over the relay: the request with the pipe's
 * bytes, then the live stdin as it arrives, while stdout and stderr stream
 * back into the invocation's writers until the exit.
 */
export function relayRpc(socketPath: string, ids: { agentSessionId: string; shellId: string }): RpcTransport {
  return async (invocation) => {
    const socket = await connectUnix(socketPath)
    const abort = () => socket.close()
    invocation.signal.addEventListener('abort', abort, { once: true })
    try {
      await socket.write(
        frameOf(codec, {
          type: 'rpc',
          ...ids,
          root: invocation.root,
          path: invocation.path,
          argv: invocation.argv,
          args: invocation.args,
          json: invocation.json,
          cwd: invocation.cwd,
          env: invocation.env,
          stdin: invocation.stdin,
        } satisfies RelayRequest),
      )
      // The live stdin may never end (it is the job's own); it is forwarded
      // for as long as the call runs and abandoned with the socket at exit.
      void (async () => {
        try {
          for await (const bytes of invocation.stdinStream) {
            if (bytes.byteLength > 0) await socket.write(frameOf(codec, { type: 'stdin', bytes } satisfies RelayRequest))
          }
          await socket.write(frameOf(codec, { type: 'stdin_end' } satisfies RelayRequest))
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
