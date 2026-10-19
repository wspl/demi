/**
 * A peer connection and its data channels as a test plays them, for the
 * tests of the direct channel's operations (`peer.test.ts`,
 * `operations.test.ts`).
 */
import type { ChannelMaker, OperationDataChannel } from './peer'

/** A data channel the test opens, closes and answers on. */
export class PlayedChannel extends EventTarget implements OperationDataChannel {
  readyState: RTCDataChannelState = 'connecting'
  binaryType: BinaryType = 'blob'
  bufferedAmount = 0
  bufferedAmountLowThreshold = 0
  readonly sent: string[] = []

  send(data: string | Blob | ArrayBuffer | ArrayBufferView): void {
    if (typeof data === 'string')
      this.sent.push(data)
  }

  close(): void {
    this.end()
  }

  open(): void {
    this.readyState = 'open'
    this.dispatchEvent(new Event('open'))
  }

  answer(text: string): void {
    this.dispatchEvent(new MessageEvent('message', { data: text }))
  }

  end(): void {
    if (this.readyState === 'closed')
      return
    this.readyState = 'closed'
    this.dispatchEvent(new Event('close'))
  }
}

/** A connection whose channels the test plays, in the order the page makes them. */
export class PlayedConnection implements ChannelMaker {
  connectionState: RTCPeerConnectionState = 'connected'
  readonly channels: PlayedChannel[] = []
  /** Each channel's label and options, as the page made it. */
  readonly inits: [string, RTCDataChannelInit][] = []
  private made = Promise.withResolvers<void>()

  createDataChannel(label: string, init: RTCDataChannelInit): PlayedChannel {
    const channel = new PlayedChannel()
    this.channels.push(channel)
    this.inits.push([label, init])
    this.made.resolve()
    this.made = Promise.withResolvers()
    return channel
  }

  /** The page's channel number `index`, once it made it. */
  async channel(index: number): Promise<PlayedChannel> {
    while (!this.channels[index])
      await this.made.promise
    return this.channels[index]
  }
}
