/**
 * A device's signaling socket (`web-api.md` § Direct channel): the page's
 * introduction to the device's runner. It carries one offer at a time and
 * its answer, each side's candidates found after them, and the page's
 * probes of the relay path with their answers; a socket that brings nothing, not even a heartbeat, for as
 * long as the page's sockets may is broken, and a closed or broken socket
 * connects again after the page's reconnect waits (`web-application.md`
 * § Liveness and reconnection). Its close closes the runner's peer, and the
 * backend closes the peer too when the user turns a plugin on or off, which
 * the socket says so that the page offers again at once.
 */
import { z } from 'zod'
import { reportError } from '@demicodes/web-ui/infra/errors'
import {
  openSocket,
  waitToReconnect,
  watchSilence,
  type ReconnectWait,
  type SilenceWatch,
} from '@demicodes/web-ui/transport/liveness'
import { apiUrl } from '../api/client'
import { directMessageSchema, type DirectMessage, type DirectRequest } from '../api/generated/web-api'
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

export class DeviceSignaling implements PeerSignaling {
  private socket: WebSocket | null = null
  private silence: SilenceWatch | null = null
  private waiting: ReconnectWait | null = null
  private failures = 0
  private stopped = false
  /** The offer waiting for its answer. */
  private answering: PromiseWithResolvers<string> | null = null
  /** Who hears the runner's candidates. */
  private readonly listeners = new Set<(candidate: string) => void>()
  /** Who hears the runner's answers to the relay probes. */
  private readonly pongs = new Set<(id: number) => void>()

  /**
   * @param deviceId The paired device the socket introduces the page to.
   * @param opened Called each time the socket opens: first, and again after it closed.
   * @param peerClosed Called when the backend closed the runner's peer, whose introduction is out of date.
   */
  constructor(
    private readonly deviceId: string,
    private readonly opened: () => void,
    private readonly peerClosed: () => void,
  ) {
    this.connect()
  }

  /** Sends an offer and answers the runner's answer; fails when the socket is not open or closes first. */
  offer(sdp: string): Promise<string> {
    const socket = this.socket
    if (!socket || socket.readyState !== WebSocket.OPEN)
      return Promise.reject(new ChannelFailed('The signaling socket is not open'))
    this.answering?.reject(new ChannelFailed('Another offer replaced this one'))
    this.answering = Promise.withResolvers()
    const request: DirectRequest = { type: 'offer', sdp }
    socket.send(JSON.stringify(request))
    return this.answering.promise
  }

  /** Sends a candidate the browser found after the offer; one for a socket that closed meanwhile has no peer left. */
  candidate(candidate: string): void {
    const socket = this.socket
    if (!socket || socket.readyState !== WebSocket.OPEN)
      return
    const request: DirectRequest = { type: 'candidate', candidate }
    socket.send(JSON.stringify(request))
  }

  /** Sends a probe of the relay path; one for a socket that is not open is lost. */
  ping(id: number): void {
    const socket = this.socket
    if (!socket || socket.readyState !== WebSocket.OPEN)
      return
    const request: DirectRequest = { type: 'ping', id }
    socket.send(JSON.stringify(request))
  }

  /** Calls `listener` with the id of each probe the runner answered; the answer stops it. */
  onPong(listener: (id: number) => void): () => void {
    this.pongs.add(listener)
    return () => this.pongs.delete(listener)
  }

  candidates(listener: (candidate: string) => void): () => void {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
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
    const socket = openSocket(url)
    this.socket = socket
    let opened = false
    socket.addEventListener('open', () => {
      if (this.socket !== socket)
        return
      opened = true
      this.silence = watchSilence(() => this.lose(socket))
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
      reportError('Could not read a message of the direct channel.', z.prettifyError(parsed.error))
      socket.close()
      return
    }
    this.answer(parsed.data)
  }

  private answer(message: DirectMessage): void {
    if (message.type === 'heartbeat')
      return
    if (message.type === 'closed') {
      this.peerClosed()
      return
    }
    if (message.type === 'pong') {
      for (const listener of [...this.pongs])
        listener(message.id)
      return
    }
    if (message.type === 'candidate') {
      for (const listener of [...this.listeners])
        listener(message.candidate)
      return
    }
    const answering = this.answering
    this.answering = null
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
    this.failures = Math.max(this.failures, 1)
    this.waiting = waitToReconnect(this.failures, () => {
      this.waiting = null
      this.connect()
    })
  }

  private drop(): void {
    this.silence?.stop()
    this.silence = null
    this.answering?.reject(new ChannelFailed('The signaling socket closed'))
    this.answering = null
    const socket = this.socket
    this.socket = null
    socket?.close()
  }
}
