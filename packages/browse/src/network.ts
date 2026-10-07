// The slot's network (browse.md § Conditions): the browser reaches
// the web app through this forwarder, which passes each of the page's
// connections, requests and sockets alike, on to the web server, and adds
// what `net` sets: latency, a bandwidth limit, an offline spell, or a cut.
// The browser's own network emulation cannot do these to a WebSocket, and
// cannot end a connection without a close.
//
// Latency and bandwidth apply to the app's traffic with its backend, the
// requests and sockets under `/api`, and not to the files the dev server
// serves: a built app loads a few of them once, while the dev server's
// hundreds of modules would take minutes to load at a far backend's pace.
import net from 'node:net'
import { matcher } from './pattern'

export interface Conditions {
  /** The round trip it adds, in milliseconds: half on each way. */
  latencyMs: number
  /** The limit on each way, in kilobits per second; null for none. */
  kbps: number | null
  offline: boolean
}

/** One of the page's connections and what it has carried. */
interface Connection {
  id: number
  opened: number
  client: net.Socket
  server: net.Socket
  /** The requests seen on it, `GET /api/sync` and so on, the latest last. */
  requests: string[]
  /** Whether a request upgraded it to a WebSocket. */
  socket: boolean
  /** Whether its latest request is the app's with its backend, which latency and bandwidth slow. */
  paced: boolean
  lanes: Lane[]
}

/** How much a way may hold before it stops reading from its sender. */
const HIGH_WATER = 4 * 1024 * 1024

/**
 * One way of a connection: chunks from `from`, delivered to `to` once the
 * conditions allow, in order.
 */
class Lane {
  private readonly queue: { due: number, chunk: Buffer }[] = []
  private queued = 0
  /** When the way is free again under a bandwidth limit. */
  private freeAt = 0
  private timer: ReturnType<typeof setTimeout> | null = null
  /** Whether `from` has ended its way, which ends `to`'s once delivered. */
  private ended = false

  constructor(
    private readonly from: net.Socket,
    private readonly to: net.Socket,
    private readonly conditions: () => Conditions,
  ) {
    from.on('data', (chunk: Buffer) => this.take(chunk))
    from.on('end', () => {
      this.ended = true
      this.pump()
    })
  }

  private take(chunk: Buffer): void {
    this.queue.push({ due: Date.now() + this.conditions().latencyMs / 2, chunk })
    this.queued += chunk.length
    if (this.queued > HIGH_WATER) {
      this.from.pause()
    }
    this.pump()
  }

  /** Delivers what is due, and waits for the next chunk's time. */
  pump(): void {
    if (this.timer) {
      clearTimeout(this.timer)
      this.timer = null
    }
    const conditions = this.conditions()
    if (conditions.offline) {
      return
    }
    while (this.queue.length > 0) {
      const head = this.queue[0]
      const at = Math.max(head.due, this.freeAt)
      const now = Date.now()
      if (at > now) {
        this.timer = setTimeout(() => this.pump(), at - now)
        return
      }
      this.queue.shift()
      this.queued -= head.chunk.length
      if (conditions.kbps !== null) {
        this.freeAt = Math.max(now, this.freeAt) + (head.chunk.length * 8) / conditions.kbps
      }
      if (!this.to.destroyed) {
        this.to.write(head.chunk)
      }
    }
    if (this.from.isPaused() && this.queued <= HIGH_WATER) {
      this.from.resume()
    }
    if (this.ended && !this.to.destroyed) {
      this.to.end()
    }
  }

  stop(): void {
    if (this.timer) {
      clearTimeout(this.timer)
      this.timer = null
    }
  }
}

const REQUEST_LINE = /^(GET|HEAD|POST|PUT|PATCH|DELETE|OPTIONS) (\S+) HTTP\/1\.[01]\r\n/

export class Network {
  private readonly conditions: Conditions = { latencyMs: 0, kbps: null, offline: false }
  private readonly connections = new Map<number, Connection>()
  private nextId = 1
  private readonly server: net.Server

  private constructor(server: net.Server) {
    this.server = server
  }

  /** Listens on `port` and forwards each connection to `target` on this machine. */
  static async listen(port: number, target: number): Promise<Network> {
    // Each way ends on its own, once what it holds is delivered.
    const server = net.createServer({ allowHalfOpen: true })
    const network = new Network(server)
    server.on('connection', (client) => network.accept(client, target))
    await new Promise<void>((resolve, reject) => {
      server.once('error', reject)
      server.listen(port, '127.0.0.1', () => {
        server.off('error', reject)
        resolve()
      })
    })
    return network
  }

  private accept(client: net.Socket, target: number): void {
    // Offline, a new connection fails as one to an unreachable server does.
    if (this.conditions.offline) {
      client.resetAndDestroy()
      return
    }
    const server = net.connect({ port: target, host: '127.0.0.1', allowHalfOpen: true })
    const connection: Connection = {
      id: this.nextId,
      opened: Date.now(),
      client,
      server,
      requests: [],
      socket: false,
      paced: false,
      lanes: [],
    }
    this.nextId += 1
    this.connections.set(connection.id, connection)
    client.on('data', (chunk: Buffer) => this.observe(connection, chunk))
    const conditions = (): Conditions => connection.paced
      ? this.conditions
      : { latencyMs: 0, kbps: null, offline: this.conditions.offline }
    connection.lanes = [new Lane(client, server, conditions), new Lane(server, client, conditions)]
    // A failure on either side ends both at once, as a broken connection
    // does; the connection is gone once both sides have closed.
    let open = 2
    for (const side of [client, server]) {
      side.on('error', () => this.close(connection))
      side.on('close', () => {
        open -= 1
        if (open === 0) {
          this.close(connection)
        }
      })
    }
  }

  /** Notes the requests a chunk from the page starts. */
  private observe(connection: Connection, chunk: Buffer): void {
    const head = chunk.subarray(0, 2048).toString('latin1')
    const line = REQUEST_LINE.exec(head)
    if (!line) {
      return
    }
    connection.requests.push(`${line[1]} ${line[2]}`)
    connection.paced = line[2].startsWith('/api/')
    if (/\r\nupgrade:\s*websocket/i.test(head)) {
      connection.socket = true
    }
  }

  private close(connection: Connection): void {
    if (!this.connections.delete(connection.id)) {
      return
    }
    for (const lane of connection.lanes) {
      lane.stop()
    }
    connection.client.destroy()
    connection.server.destroy()
  }

  /** The port it listens on, which a port 0 lets the system choose. */
  get port(): number {
    const address = this.server.address()
    if (address === null || typeof address === 'string') {
      throw new Error('the network does not listen on a TCP port')
    }
    return address.port
  }

  get state(): Readonly<Conditions> {
    return this.conditions
  }

  set(change: Partial<Conditions>): void {
    Object.assign(this.conditions, change)
    // A lane holds what an offline spell kept, and its timers follow the old
    // latency: each looks again under the new conditions.
    for (const connection of this.connections.values()) {
      for (const lane of connection.lanes) {
        lane.pump()
      }
    }
  }

  /**
   * Ends every connection whose requests match `pattern` with a reset, as a
   * lost connection does: the page's socket closes without a close frame.
   * Answers what it cut.
   */
  cut(pattern: string): string[] {
    const matches = matcher(pattern)
    const cut: string[] = []
    for (const connection of [...this.connections.values()]) {
      if (!connection.requests.some(matches)) {
        continue
      }
      cut.push(describe(connection))
      connection.client.resetAndDestroy()
      connection.server.resetAndDestroy()
      this.close(connection)
    }
    return cut
  }

  /** The open connections, one line each. */
  list(): string[] {
    return [...this.connections.values()].map(describe)
  }

  /** Ends every connection and stops listening. */
  async shutdown(): Promise<void> {
    for (const connection of [...this.connections.values()]) {
      this.close(connection)
    }
    await new Promise<void>((resolve) => this.server.close(() => resolve()))
  }
}

function describe(connection: Connection): string {
  const kind = connection.socket ? 'socket' : 'http'
  const last = connection.requests.at(-1) ?? '(no request yet)'
  const age = ((Date.now() - connection.opened) / 1000).toFixed(1)
  return `${kind} ${last} (open ${age} s)`
}
