import { deferred, type Deferred } from '@demicodes/utils'

type Phase = { kind: 'idle' } | { kind: 'waiting'; next: Deferred<IteratorResult<Uint8Array>> } | { kind: 'closed' }

/** One explicitly requested live-stdin chunk, without a speculative input queue. */
export class LiveInput {
  private phase: Phase = { kind: 'idle' }

  constructor(private readonly pull: () => void) {}

  push(bytes: Uint8Array): void {
    if (this.phase.kind === 'closed')
      return
    if (this.phase.kind !== 'waiting' || bytes.length > 64 * 1024)
      throw new Error('Unrequested or oversized RPC stdin chunk')
    const next = this.phase.next
    this.phase = { kind: 'idle' }
    next.resolve({ done: false, value: bytes })
  }

  close(): void {
    const previous = this.phase
    this.phase = { kind: 'closed' }
    if (previous.kind === 'waiting')
      previous.next.resolve({ done: true, value: undefined })
  }

  stream(): AsyncIterable<Uint8Array> {
    return {
      [Symbol.asyncIterator]: () => ({
        next: () => this.next(),
        return: async () => {
          this.close()
          return { done: true, value: undefined }
        },
      }),
    }
  }

  private next(): Promise<IteratorResult<Uint8Array>> {
    if (this.phase.kind === 'closed')
      return Promise.resolve({ done: true, value: undefined })
    if (this.phase.kind === 'waiting')
      return Promise.reject(new Error('Concurrent RPC stdin readers'))
    const next = deferred<IteratorResult<Uint8Array>>()
    this.phase = { kind: 'waiting', next }
    try {
      this.pull()
    } catch (error) {
      this.phase = { kind: 'closed' }
      next.reject(error)
    }
    return next.promise
  }
}
