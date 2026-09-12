// The backend's end of the machine wire: a `ManagedHostProvisioner` whose
// every call travels to the manager over its Unix socket. The connection is
// opened by the first call and reopened by the next call after it drops;
// calls in flight when it drops fail, and the lifecycle retries them the
// way it retries any failed operation. `close` reconciles the manager (every
// machine off, disks saved) and disconnects, so a backend stop leaves the
// same state an in-process provisioner would; the manager keeps running.
import { errorMessage } from '@demicodes/utils'
import {
  machineOps,
  machineResponseSchema,
  type MachineOp,
  type MachineParams,
  type MachineRequest,
  type MachineResult
} from './protocol'
import type {
  BootArgs,
  MachineImageState,
  ManagedHostProvisioner,
  ManagedVolume
} from './provisioner'
import { encodeFrame, FrameWriter, LineReader } from './wire'

export interface RemoteProvisionerOptions {
  socketPath: string
  log?: (line: string) => void
}

interface Pending {
  op: MachineOp
  resolve: (result: unknown) => void
  reject: (error: Error) => void
}

interface Connection {
  socket: Bun.Socket<undefined>
  writer: FrameWriter
}

export class RemoteProvisioner implements ManagedHostProvisioner {
  private connection: Promise<Connection> | null = null
  private readonly pending = new Map<string, Pending>()
  private readonly deathListeners: Array<(deviceId: string) => void> = []
  private readonly log: (line: string) => void
  private nextId = 1

  constructor(private readonly options: RemoteProvisionerOptions) {
    this.log = options.log ?? console.warn
  }

  reconcile(): Promise<void> {
    return this.call('reconcile', {}).then(() => undefined)
  }

  currentBaseVersion(): Promise<string> {
    return this.call('current_base_version', {})
  }

  imageState(deviceId: string): Promise<MachineImageState | null> {
    return this.call('image_state', { deviceId })
  }

  wake(deviceId: string, boot: BootArgs): Promise<void> {
    return this.call('wake', { deviceId, boot }).then(() => undefined)
  }

  hibernate(deviceId: string): Promise<void> {
    return this.call('hibernate', { deviceId }).then(() => undefined)
  }

  checkpoint(deviceId: string): Promise<void> {
    return this.call('checkpoint', { deviceId }).then(() => undefined)
  }

  growVolume(
    deviceId: string,
    volume: ManagedVolume,
    bytes: number
  ): Promise<void> {
    return this.call('grow_volume', { deviceId, volume, bytes })
      .then(() => undefined)
  }

  reset(
    deviceId: string,
    operationId: string,
    baseVersion: string
  ): Promise<void> {
    return this.call('reset', { deviceId, operationId, baseVersion })
      .then(() => undefined)
  }

  onDeath(listener: (deviceId: string) => void): void {
    this.deathListeners.push(listener)
  }

  async close(): Promise<void> {
    if (!this.connection) {
      return
    }
    // A manager that cannot be reached at shutdown is logged, not fatal: the
    // backend still stops, and the next start reconciles.
    await this.reconcile().catch(error => {
      this.log(`machines: reconcile at close failed: ${errorMessage(error)}`)
    })
    const connection = await this.connection?.catch(() => null)
    this.connection = null
    connection?.socket.end()
  }

  private async call<Op extends MachineOp>(
    op: Op,
    params: MachineParams<Op>
  ): Promise<MachineResult<Op>> {
    const connection = await this.connect()
    const id = String(this.nextId++)
    const request = { id, op, params } as MachineRequest
    const result = await new Promise<unknown>((resolve, reject) => {
      this.pending.set(id, { op, resolve, reject })
      connection.writer.write(encodeFrame(request))
    })
    return machineOps[op].result.parse(result) as MachineResult<Op>
  }

  private connect(): Promise<Connection> {
    this.connection ??= this.open().catch(error => {
      this.connection = null
      throw error
    })
    return this.connection
  }

  private async open(): Promise<Connection> {
    const reader = new LineReader()
    let writer: FrameWriter | null = null
    const socket = await Bun.connect<undefined>({
      unix: this.options.socketPath,
      socket: {
        data: (_socket, chunk) => {
          reader.push(chunk, line => this.receive(line))
        },
        drain: () => {
          writer?.drain()
        },
        close: () => {
          this.dropped('the machine manager closed the connection')
        },
        error: (_socket, error) => {
          this.dropped(errorMessage(error))
        },
        connectError: (_socket, error) => {
          this.dropped(errorMessage(error))
        },
      },
    })
    writer = new FrameWriter(socket)
    return { socket, writer }
  }

  private receive(line: string): void {
    let json: unknown
    try {
      json = JSON.parse(line)
    } catch (error) {
      this.log(`machines: unreadable frame from the manager: ${errorMessage(error)}`)
      return
    }
    const parsed = machineResponseSchema.safeParse(json)
    if (!parsed.success) {
      this.log(`machines: unexpected frame from the manager: ${parsed.error.message}`)
      return
    }
    const response = parsed.data
    if (response.type === 'death') {
      for (const listener of this.deathListeners) {
        listener(response.deviceId)
      }
      return
    }
    const pending = this.pending.get(response.id)
    if (!pending) {
      return
    }
    this.pending.delete(response.id)
    if (response.type === 'ok') {
      pending.resolve(response.result)
    } else {
      pending.reject(new Error(response.message))
    }
  }

  /** The connection is gone: every call in flight fails, the next call reconnects. */
  private dropped(reason: string): void {
    this.connection = null
    if (this.pending.size === 0) {
      return
    }
    this.log(`machines: ${reason}; ${this.pending.size} call(s) failed`)
    const failed = [...this.pending.values()]
    this.pending.clear()
    for (const call of failed) {
      call.reject(new Error(`Machine manager unavailable during ${call.op}: ${reason}`))
    }
  }
}
