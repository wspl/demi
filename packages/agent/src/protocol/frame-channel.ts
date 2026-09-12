export class AgentTransportError extends Error {
  constructor(
    readonly code: 'invalid_frame' | 'transport_error' | 'closed' | 'handler_failed',
    message: string,
  ) {
    super(message)
    this.name = 'AgentTransportError'
  }
}

/** Ordered delivery and error subscriptions shared by the transport adapters. */
export class FrameChannel<Frame> {
  private readonly handlers = new Set<(frame: Frame) => void | Promise<void>>()
  private readonly errors = new Set<(error: AgentTransportError) => void>()
  private readonly queue: Array<{ frame: Frame; resolve: () => void }> = []
  private status: 'idle' | 'running' | 'closed' = 'idle'

  onFrame(handler: (frame: Frame) => void | Promise<void>): () => void {
    this.handlers.add(handler)
    return () => {
      this.handlers.delete(handler)
    }
  }

  onError(handler: (error: AgentTransportError) => void): () => void {
    this.errors.add(handler)
    return () => {
      this.errors.delete(handler)
    }
  }

  receive(frame: Frame): Promise<void> {
    if (this.status === 'closed') return Promise.resolve()
    return new Promise((resolve) => {
      this.queue.push({ frame, resolve })
      if (this.status === 'idle') void this.drain()
    })
  }

  private async drain(): Promise<void> {
    this.status = 'running'
    while (this.queue.length > 0) {
      const entry = this.queue.shift()!
      try {
        for (const handler of this.handlers) {
          // Deliver synchronous handlers without adding a microtask per frame.
          // A takeover's close must precede another connection's open result.
          const result = handler(entry.frame)
          if (result && typeof result.then === 'function') await result
        }
      } catch {
        this.report(new AgentTransportError('handler_failed', 'Agent frame handler failed'))
      } finally {
        entry.resolve()
      }
    }
    if (this.status === 'running') this.status = 'idle'
  }

  report(error: AgentTransportError): void {
    if (this.status === 'closed') return
    for (const handler of [...this.errors]) handler(error)
  }

  close(): void {
    this.status = 'closed'
    this.handlers.clear()
    this.errors.clear()
    for (const entry of this.queue.splice(0)) entry.resolve()
  }
}
