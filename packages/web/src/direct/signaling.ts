/**
 * A device's signaling socket (`web-api-hosts.md` § Direct channel): the page's
 * introduction to the device's runner, for both of the page's peers with it,
 * `files` and `live` (`direct-channel.md` § Making the channel), each
 * message naming its peer. It carries one offer at a time for each peer and
 * its answer, each side's candidates found after them, and the page's
 * probes of the relay path with their answers; a socket that brings nothing, not even a heartbeat, for as
 * long as the page's sockets may is broken, and a closed or broken socket
 * connects again after the page's reconnect waits (`web-application.md`
 * § Liveness and reconnection). Its close closes the runner's peers, and the
 * backend closes a peer too when the user turns a plugin on or off, which
 * the socket says so that the page offers again at once.
 */
import { schemaRefusal } from '@demicodes/protocol'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { t } from './locales/text'
import {
  openSocket,
  pingOn,
  waitToReconnect,
  watchSilence,
  type ReconnectWait,
  type SilenceWatch,
} from '@demicodes/web-ui/transport/liveness'
import { apiUrl } from '../api/client'
import { directMessageSchema, type DirectMessage, type DirectPeer as PeerName, type DirectRequest } from '../api/generated/web-api'
import { ChannelFailed, type PeerSignaling } from './peer'

/** An offer the runner did not answer. */
export class Unanswered extends ChannelFailed {
  constructor(readonly code: string) {
    super(`The runner did not answer the offer: ${code}`)
    this.name = 'Unanswered'
  }
}

/** Whether `error` is the runner's refusal of an offer for having too many peers. */
export function isBusy(error: unknown): boolean {
  return error instanceof Unanswered && error.code === 'busy'
}

/** What the socket holds for one of the page's peers. */
class PeerSlot {
  /** The offer waiting for its answer. */
  answering: PromiseWithResolvers<string> | null = null
  /** Who hears the runner's candidates. */
  readonly listeners = new Set<(candidate: string) => void>()

  /** The offer waiting, if any, fails with `error`. */
  fail(error: Error): void {
    this.answering?.reject(error)
    this.answering = null
  }
}

export class DeviceSignaling {
  private socket: WebSocket | null = null
  private silence: SilenceWatch | null = null
  private waiting: ReconnectWait | null = null
  private failures = 0
  private stopped = false
  private readonly peers: Record<PeerName, PeerSlot> = { files: new PeerSlot(), live: new PeerSlot() }
  /** Who hears the runner's answers to the relay probes. */
  private readonly pongs = new Set<(id: number) => void>()

  /**
   * @param deviceId The paired device the socket introduces the page to.
   * @param opened Called each time the socket opens: first, and again after it closed.
   * @param peerClosed Called when the backend closed the runner's peer `peer`, whose introduction is out of date.
   * @param dropped Called when the socket closed or was taken as broken: the backend closes the runner's peers then, so the page's peers are gone too.
   */
  constructor(
    private readonly deviceId: string,
    private readonly opened: () => void,
    private readonly peerClosed: (peer: PeerName) => void,
    private readonly dropped: () => void,
  ) {
    this.connect()
  }

  /** The signaling of the page's peer `peer`, as an attempt to make it uses it. */
  peer(peer: PeerName): PeerSignaling {
    return {
      offer: (sdp) => this.offer(peer, sdp),
      candidate: (candidate) => this.send({ type: 'candidate', peer, candidate }),
      candidates: (listener) => {
        const listeners = this.peers[peer].listeners
        listeners.add(listener)
        return () => listeners.delete(listener)
      },
    }
  }

  /** Tells the backend that the page closed its peer `peer`, so that the runner's closes too. */
  close(peer: PeerName): void {
    this.peers[peer].fail(new ChannelFailed('The page closed the peer'))
    this.send({ type: 'close', peer })
  }

  /** Sends an offer and answers the runner's answer; fails when the socket is not open or closes first. */
  private offer(peer: PeerName, sdp: string): Promise<string> {
    const socket = this.socket
    if (!socket || socket.readyState !== WebSocket.OPEN)
      return Promise.reject(new ChannelFailed('The signaling socket is not open'))
    const slot = this.peers[peer]
    slot.fail(new ChannelFailed('Another offer replaced this one'))
    const answering = Promise.withResolvers<string>()
    slot.answering = answering
    this.send({ type: 'offer', peer, sdp })
    return answering.promise
  }

  /** Sends `request`; one for a socket that is not open has no peer left, or is a probe that is lost. */
  private send(request: DirectRequest): void {
    const socket = this.socket
    if (!socket || socket.readyState !== WebSocket.OPEN)
      return
    socket.send(JSON.stringify(request))
  }

  /** Sends a probe of the relay path; one for a socket that is not open is lost. */
  ping(id: number): void {
    this.send({ type: 'ping', id })
  }

  /** Calls `listener` with the id of each probe the runner answered; the answer stops it. */
  onPong(listener: (id: number) => void): () => void {
    this.pongs.add(listener)
    return () => this.pongs.delete(listener)
  }

  /** Whether the socket is open, so an offer can go. */
  get open(): boolean {
    return this.socket?.readyState === WebSocket.OPEN
  }

  /** Closes the socket for good. */
  stop(): void {
    this.stopped = true
    this.waiting?.cancel()
    this.waiting = null
    this.drop()
  }

  private connect(): void {
    const url = new URL(apiUrl(`/devices/${encodeURIComponent(this.deviceId)}/direct`), window.location.href)
    url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
    const socket = openSocket(url, 'direct channel signaling socket')
    this.socket = socket
    let opened = false
    socket.addEventListener('open', () => {
      if (this.socket !== socket)
        return
      opened = true
      this.silence = watchSilence('direct channel signaling socket', () => this.lose(socket), pingOn(socket))
      this.failures = 0
      this.opened()
    })
    socket.addEventListener('message', (event: MessageEvent) => {
      if (this.socket !== socket)
        return
      this.silence?.heard()
      this.receive(socket, event.data)
    })
    socket.addEventListener('close', () => {
      if (!opened && this.socket === socket)
        this.failures += 1
      this.lose(socket)
    })
  }

  private receive(socket: WebSocket, data: unknown): void {
    let value: unknown = null
    try {
      value = typeof data === 'string' ? JSON.parse(data) : null
    } catch {
      value = null
    }
    const parsed = directMessageSchema.safeParse(value)
    if (!parsed.success) {
      // What did not read is for a developer; the socket starts again.
      reportError(t('failures.readChannel'), schemaRefusal(parsed.error))
      socket.close()
      return
    }
    this.answer(parsed.data)
  }

  private answer(message: DirectMessage): void {
    if (message.type === 'heartbeat')
      return
    if (message.type === 'closed') {
      this.peerClosed(message.peer)
      return
    }
    if (message.type === 'pong') {
      // The backend's answer to the return's ping and the runner's to a
      // relay probe are the same message; each side takes the ids it sent.
      this.silence?.ponged(message.id)
      for (const listener of [...this.pongs])
        listener(message.id)
      return
    }
    if (message.type === 'candidate') {
      for (const listener of [...this.peers[message.peer].listeners])
        listener(message.candidate)
      return
    }
    const slot = this.peers[message.peer]
    const answering = slot.answering
    slot.answering = null
    if (message.type === 'answer')
      answering?.resolve(message.sdp)
    else
      answering?.reject(new Unanswered(message.code))
  }

  /** The socket closed, or is taken as broken: it connects again after its wait. */
  private lose(socket: WebSocket): void {
    if (this.socket !== socket)
      return
    this.drop()
    if (this.stopped)
      return
    this.dropped()
    this.failures = Math.max(this.failures, 1)
    this.waiting = waitToReconnect(this.failures, () => {
      this.waiting = null
      this.connect()
    })
  }

  private drop(): void {
    this.silence?.stop()
    this.silence = null
    for (const slot of Object.values(this.peers))
      slot.fail(new ChannelFailed('The signaling socket closed'))
    const socket = this.socket
    this.socket = null
    socket?.close()
  }
}
