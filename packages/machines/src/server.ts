// The machine manager's service end: one Unix socket, any number of backend
// connections, every request dispatched to one `ManagedHostProvisioner`.
// Requests on a connection run concurrently; the provisioner serializes
// per device itself. Deaths go to every connection. No authentication: the
// socket file's permissions (owner only) are the boundary, and nothing
// listens on TCP.
import { chmod, mkdir, rm } from 'node:fs/promises'
import { dirname } from 'node:path'
import { errorMessage } from '@demicodes/utils'
import {
  machineRequestSchema,
  type MachineRequest,
  type MachineResponse
} from './protocol'
import type { ManagedHostProvisioner } from './provisioner'
import { encodeFrame, FrameWriter, LineReader } from './wire'

export interface MachineServerOptions {
  socketPath: string
  provisioner: ManagedHostProvisioner
  log?: (line: string) => void
}

export interface MachineServer {
  socketPath: string
  /** Stops listening and drops every connection; the provisioner is the caller's. */
  close(): Promise<void>
}

interface Connection {
  reader: LineReader
  writer: FrameWriter
}

export async function serveMachines(
  options: MachineServerOptions
): Promise<MachineServer> {
  const log = options.log ?? console.warn
  const connections = new Set<Bun.Socket<Connection>>()
  // Nothing is written to a connection that has gone: a request may finish
  // after its socket closed, and lines behind a bad frame are not served.
  const send = (socket: Bun.Socket<Connection>, response: MachineResponse) => {
    if (connections.has(socket)) {
      socket.data.writer.write(encodeFrame(response))
    }
  }
  const handleLine = async (socket: Bun.Socket<Connection>, line: string) => {
    if (!connections.has(socket)) {
      return
    }
    const request = parseRequest(line)
    if (!request.ok) {
      log(`machines: dropping a connection over a bad frame: ${request.message}`)
      connections.delete(socket)
      socket.end()
      return
    }
    try {
      const result = await dispatch(options.provisioner, request.value)
      send(socket, { type: 'ok', id: request.value.id, result })
    } catch (error) {
      send(socket, { type: 'error', id: request.value.id, message: errorMessage(error) })
    }
  }

  options.provisioner.onDeath(deviceId => {
    for (const socket of connections) {
      send(socket, { type: 'death', deviceId })
    }
  })

  // A socket file left by an earlier process would refuse the bind.
  await mkdir(dirname(options.socketPath), { recursive: true })
  await rm(options.socketPath, { force: true })
  const listener = Bun.listen<Connection>({
    unix: options.socketPath,
    socket: {
      open(socket) {
        socket.data = { reader: new LineReader(), writer: new FrameWriter(socket) }
        connections.add(socket)
      },
      data(socket, chunk) {
        socket.data.reader.push(chunk, line => {
          void handleLine(socket, line)
        })
      },
      drain(socket) {
        socket.data.writer.drain()
      },
      close(socket) {
        connections.delete(socket)
      },
      error(socket, error) {
        connections.delete(socket)
        log(`machines: connection error: ${errorMessage(error)}`)
      },
    },
  })
  await chmod(options.socketPath, 0o600)

  return {
    socketPath: options.socketPath,
    async close() {
      listener.stop(true)
      await rm(options.socketPath, { force: true })
    },
  }
}

type Parsed =
  | { ok: true; value: MachineRequest }
  | { ok: false; message: string }

function parseRequest(line: string): Parsed {
  let json: unknown
  try {
    json = JSON.parse(line)
  } catch (error) {
    return { ok: false, message: errorMessage(error) }
  }
  const parsed = machineRequestSchema.safeParse(json)
  if (!parsed.success) {
    return { ok: false, message: parsed.error.message }
  }
  return { ok: true, value: parsed.data }
}

/** Runs one request against the provisioner; the result matches `machineOps[op].result`. */
async function dispatch(
  provisioner: ManagedHostProvisioner,
  request: MachineRequest
): Promise<unknown> {
  switch (request.op) {
    case 'reconcile':
      await provisioner.reconcile()
      return null
    case 'current_base_version':
      return provisioner.currentBaseVersion()
    case 'image_state':
      return provisioner.imageState(request.params.deviceId)
    case 'wake':
      await provisioner.wake(request.params.deviceId, request.params.boot)
      return null
    case 'hibernate':
      await provisioner.hibernate(request.params.deviceId)
      return null
    case 'checkpoint':
      await provisioner.checkpoint(request.params.deviceId)
      return null
    case 'grow_volume':
      await provisioner.growVolume(
        request.params.deviceId,
        request.params.volume,
        request.params.bytes
      )
      return null
    case 'reset':
      await provisioner.reset(
        request.params.deviceId,
        request.params.operationId,
        request.params.baseVersion
      )
      return null
    default:
      return unreachable(request)
  }
}

function unreachable(request: never): never {
  throw new Error(`Unhandled machine request ${JSON.stringify(request)}`)
}
