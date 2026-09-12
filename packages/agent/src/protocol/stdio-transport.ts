import type { Readable, Writable } from 'node:stream'
import type { ClientFrame, ServerFrame } from './frames'
import { parsePortableJson, stringifyPortableJson } from '@demicodes/utils'
import type { AgentTransport, AgentClientTransport, AgentServerTransport } from './transport'
import { AgentTransportError, FrameChannel } from './frame-channel'
import { clientFrameSchema } from './schemas'
import { serverFrameSchema } from './server-schemas'

export function createStdioClientTransport(
  readable: Readable,
  writable: Writable,
): AgentClientTransport {
  return new JsonLineTransport<ClientFrame, ServerFrame>(
    readable,
    writable,
    serverFrameSchema.parse,
  )
}

export function createStdioServerTransport(
  readable: Readable,
  writable: Writable,
): AgentServerTransport {
  return new JsonLineTransport<ServerFrame, ClientFrame>(
    readable,
    writable,
    clientFrameSchema.parse,
  )
}

class JsonLineTransport<Send, Receive> implements AgentTransport<Send, Receive> {
  private readonly channel = new FrameChannel<Receive>()
  private closed = false
  private readonly onWriteError = (): void => {
    this.channel.report(new AgentTransportError('transport_error', 'Agent output stream failed'))
    this.close()
  }

  constructor(
    private readonly readable: Readable,
    private readonly writable: Writable,
    private readonly decode: (value: unknown) => Receive,
  ) {
    writable.on('error', this.onWriteError)
    void this.readFrames()
  }

  private async readFrames(): Promise<void> {
    const decoder = new TextDecoder('utf-8', { fatal: true })
    let buffer = ''
    try {
      for await (const chunk of this.readable) {
        if (this.closed) return
        if (!(chunk instanceof Uint8Array)) throw new Error('Agent input stream must provide bytes')
        buffer += decoder.decode(chunk, { stream: true })
        let newline = buffer.indexOf('\n')
        while (newline !== -1) {
          await this.receiveLine(buffer.slice(0, newline))
          buffer = buffer.slice(newline + 1)
          newline = buffer.indexOf('\n')
        }
      }
      buffer += decoder.decode()
      if (buffer.length > 0) await this.receiveLine(buffer)
      this.channel.report(new AgentTransportError('closed', 'Agent input stream closed'))
    } catch {
      // Destroying our readable during close also rejects its async iterator.
      if (!this.closed)
        this.channel.report(new AgentTransportError('transport_error', 'Agent input stream failed'))
    } finally {
      this.close()
    }
  }

  private async receiveLine(line: string): Promise<void> {
    if (this.closed || line.trim() === '') return
    let frame: Receive
    try {
      frame = this.decode(parsePortableJson(line))
    } catch {
      this.channel.report(new AgentTransportError('invalid_frame', 'Invalid agent frame'))
      return
    }
    await this.channel.receive(frame)
  }

  send(frame: Send): void {
    if (this.closed) throw new Error('Agent transport is closed')
    this.writable.write(`${stringifyPortableJson(frame)}\n`)
  }

  onFrame(handler: (frame: Receive) => void | Promise<void>): () => void {
    return this.channel.onFrame(handler)
  }

  onError(handler: (error: AgentTransportError) => void): () => void {
    return this.channel.onError(handler)
  }

  close(): void {
    if (this.closed) return
    this.closed = true
    this.channel.close()
    this.writable.removeListener('error', this.onWriteError)
    this.readable.destroy()
    this.writable.destroy()
  }
}
