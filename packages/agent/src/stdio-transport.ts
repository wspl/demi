import { createInterface } from 'node:readline'
import type { Readable, Writable } from 'node:stream'
import type { ClientFrame, ServerFrame } from './frames'
import { parseAgentJson, stringifyAgentJson } from './json-codec'
import type { AgentTransport, AgentClientTransport, AgentServerTransport } from './transport'

export function createStdioClientTransport(readable: Readable, writable: Writable): AgentClientTransport {
  return new JsonLineTransport<ClientFrame, ServerFrame>(readable, writable)
}

export function createStdioServerTransport(readable: Readable, writable: Writable): AgentServerTransport {
  return new JsonLineTransport<ServerFrame, ClientFrame>(readable, writable)
}

class JsonLineTransport<SendFrame, ReceiveFrame> implements AgentTransport<SendFrame, ReceiveFrame> {
  private readonly handlers = new Set<(frame: ReceiveFrame) => void>()
  private readonly readline
  private closed = false

  constructor(
    private readonly readable: Readable,
    private readonly writable: Writable,
  ) {
    this.readline = createInterface({ input: readable })
    this.readline.on('line', (line) => {
      if (this.closed || line.trim() === '') return
      const frame = parseAgentJson<ReceiveFrame>(line)
      for (const handler of this.handlers) handler(frame)
    })
  }

  send(frame: SendFrame): void {
    if (this.closed) throw new Error('Agent transport is closed')
    this.writable.write(`${stringifyAgentJson(frame)}\n`)
  }

  onFrame(handler: (frame: ReceiveFrame) => void): () => void {
    this.handlers.add(handler)
    return () => {
      this.handlers.delete(handler)
    }
  }

  close(): void {
    this.closed = true
    this.handlers.clear()
    this.readline.close()
    this.readable.destroy()
    this.writable.end()
  }
}
