import { createInterface } from 'node:readline'
import type { Readable, Writable } from 'node:stream'
import type { ClientFrame, ServerFrame } from './frames'
import { parsePortableJson, stringifyPortableJson } from '@demicodes/utils'
import type {
  AgentTransport,
  AgentClientTransport,
  AgentServerTransport
} from './transport'

export function createStdioClientTransport(
  readable: Readable,
  writable: Writable
): AgentClientTransport {
  return new JsonLineTransport<ClientFrame>(readable, writable)
}

export function createStdioServerTransport(
  readable: Readable,
  writable: Writable
): AgentServerTransport {
  return new JsonLineTransport<ServerFrame>(readable, writable)
}

class JsonLineTransport<SendFrame> implements AgentTransport<SendFrame> {
  private readonly handlers = new Set<(frame: unknown) => void>()
  private readonly readline
  private closed = false

  constructor(
    private readonly readable: Readable,
    private readonly writable: Writable,
  ) {
    this.readline = createInterface({ input: readable })
    this.readline.on('line', (line) => {
      if (this.closed || line.trim() === '')
        return
      let frame: unknown
      try {
        frame = parsePortableJson(line)
      } catch {
        // A line that is not JSON breaks the framing itself, not one frame:
        // nothing after it can be trusted to start where a frame starts.
        this.close()
        return
      }
      for (const handler of this.handlers) handler(frame)
    })
  }

  send(frame: SendFrame): void {
    if (this.closed)
      throw new Error('Agent transport is closed')
    this.writable.write(`${stringifyPortableJson(frame)}\n`)
  }

  onFrame(handler: (frame: unknown) => void): () => void {
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
