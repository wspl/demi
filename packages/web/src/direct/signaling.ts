/**
 * A device's signaling socket (`web-api.md` § Direct channel): the page's
 * introduction to the device's runner. It carries one offer at a time and
 * its answer; a socket that brings nothing, not even a heartbeat, for as
 * long as the page's sockets may is broken, and a closed or broken socket
 * connects again after the page's reconnect waits (`web-application.md`
 * § Liveness and reconnection). Its close closes the runner's peer.
 */
import { z } from 'zod'
import { reportError } from '@demicodes/web-ui/infra/errors'
import {
  waitToReconnect,
  watchSilence,
  type ReconnectWait,
  type SilenceWatch,
} from '@demicodes/web-ui/transport/liveness'
import { apiUrl } from '../api/client'
import { directMessageSchema, type DirectMessage, type DirectRequest } from '../api/generated/web-api'
import { ChannelFailed } from './peer'

/** An offer the runner did not answer. */
export class Unanswered extends ChannelFailed {
  constructor(readonly code: string) {
    super(`The runner did not answer the offer: ${code}`)
    this.name = 'Unanswered'
  }
}

export class DeviceSignaling {
  private socket: WebSocket | null = null
  private silence: SilenceWatch | null = null
  private waiting: ReconnectWait | null = null
  private failures = 0
  private stopped = false
  /** The offer waiting for its answer. */
  private answering: PromiseWithResolvers<string> | null = null

  /**
   * @param deviceId The paired device the socket introduces the page to.
   * @param opened Called each time the socket opens: first, and again after it closed.
   */
  constructor(
    private readonly deviceId: string,
    private readonly opened: () => void,
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
    const socket = new WebSocket(url)
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
