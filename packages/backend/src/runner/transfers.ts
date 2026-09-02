// Brokered transfers (`runner.md` § Transfers): the backend pipes one
// runner's `PUT` into another's `GET` — or into an in-process consumer —
// without holding the body. A transfer is single-use, expires when the
// other side never shows up, and fails when either side drops.
import { deferred, errorMessage, type Deferred } from '@demicodes/utils'
import { generateDeviceToken } from './claim-codes'

/** Where a transfer's bytes go: a runner that will `GET` them, or this process. */
export type TransferDestination = { deviceId: string } | { consume(chunk: Uint8Array): Promise<void> }

export interface Transfer {
  id: string
  /** Origin-relative; runners resolve it against their backend URL. */
  url: string
  /** Resolves once the destination drained the body; rejects when the transfer failed. */
  done: Promise<void>
}

interface PendingTransfer {
  sourceDeviceId: string
  destination: TransferDestination
  body: Deferred<ReadableStream<Uint8Array>>
  drained: Deferred<void>
  timer: ReturnType<typeof setTimeout>
}

export class TransferBroker {
  private readonly transfers = new Map<string, PendingTransfer>()

  constructor(private readonly options: { timeoutMs?: number } = {}) {}

  /** Mints a transfer the source device will `PUT`. */
  open(sourceDeviceId: string, destination: TransferDestination): Transfer {
    const id = generateDeviceToken()
    const transfer: PendingTransfer = {
      sourceDeviceId,
      destination,
      body: deferred(),
      drained: deferred(),
      timer: setTimeout(() => this.fail(id, 'the other side never arrived'), this.options.timeoutMs ?? 60_000),
    }
    // Unobserved rejections on the two internal promises are expected paths.
    transfer.body.promise.catch(() => {})
    transfer.drained.promise.catch(() => {})
    this.transfers.set(id, transfer)
    if ('consume' in destination) void this.pipeInto(id, destination.consume)
    return { id, url: `/api/transfers/${id}`, done: transfer.drained.promise }
  }

  /** `PUT /api/transfers/:id` by `deviceId`: the body streams to the destination; resolves with the HTTP status once drained. */
  async put(id: string, deviceId: string, body: ReadableStream<Uint8Array> | null): Promise<{ status: number; message: string }> {
    const transfer = this.transfers.get(id)
    if (!transfer || transfer.sourceDeviceId !== deviceId) return { status: 404, message: 'no such transfer' }
    if (!body) return { status: 400, message: 'a body is required' }
    transfer.body.resolve(body)
    try {
      await transfer.drained.promise
      return { status: 200, message: 'transferred' }
    } catch (error) {
      return { status: 409, message: errorMessage(error) }
    }
  }

  /** `GET /api/transfers/:id` by `deviceId`: the body once the source arrived. Its completion settles the transfer. */
  async get(id: string, deviceId: string): Promise<{ status: 200; body: ReadableStream<Uint8Array> } | { status: number; message: string }> {
    const transfer = this.transfers.get(id)
    if (!transfer || !('deviceId' in transfer.destination) || transfer.destination.deviceId !== deviceId) {
      return { status: 404, message: 'no such transfer' }
    }
    let body: ReadableStream<Uint8Array>
    try {
      body = await transfer.body.promise
    } catch (error) {
      return { status: 409, message: errorMessage(error) }
    }
    return { status: 200, body: this.settling(id, body) }
  }

  /** Fails every transfer a device is party to — its connection dropped. */
  deviceGone(deviceId: string): void {
    for (const [id, transfer] of this.transfers) {
      const destination = transfer.destination
      if (transfer.sourceDeviceId === deviceId || ('deviceId' in destination && destination.deviceId === deviceId)) {
        this.fail(id, `device ${deviceId} disconnected`)
      }
    }
  }

  fail(id: string, reason: string): void {
    const transfer = this.transfers.get(id)
    if (!transfer) return
    this.finish(id)
    const error = new Error(`transfer failed: ${reason}`)
    transfer.body.reject(error)
    transfer.drained.reject(error)
  }

  close(): void {
    for (const id of [...this.transfers.keys()]) this.fail(id, 'backend shutting down')
  }

  private async pipeInto(id: string, consume: (chunk: Uint8Array) => Promise<void>): Promise<void> {
    const transfer = this.transfers.get(id)
    if (!transfer) return
    try {
      const reader = (await transfer.body.promise).getReader()
      for (;;) {
        const { done, value } = await reader.read()
        if (done) break
        await consume(value)
      }
      this.finish(id)
      transfer.drained.resolve()
    } catch (error) {
      this.fail(id, errorMessage(error))
    }
  }

  /** Wraps the source body so the transfer settles with the destination's read. */
  private settling(id: string, body: ReadableStream<Uint8Array>): ReadableStream<Uint8Array> {
    const reader = body.getReader()
    return new ReadableStream<Uint8Array>({
      pull: async (controller) => {
        let next: Awaited<ReturnType<typeof reader.read>>
        try {
          next = await reader.read()
        } catch (error) {
          this.fail(id, errorMessage(error))
          controller.error(error)
          return
        }
        if (next.done) {
          controller.close()
          const transfer = this.transfers.get(id)
          this.finish(id)
          transfer?.drained.resolve()
          return
        }
        controller.enqueue(next.value)
      },
      cancel: (reason) => {
        this.fail(id, `destination stopped reading: ${errorMessage(reason)}`)
        void reader.cancel(reason).catch(() => {})
      },
    })
  }

  private finish(id: string): void {
    const transfer = this.transfers.get(id)
    if (!transfer) return
    clearTimeout(transfer.timer)
    this.transfers.delete(id)
  }
}
