import { expect, test } from 'bun:test'
import { z } from 'zod'
import type { ChannelHeader } from '../api/generated/web-api'
import { ChannelFailed, openOperation, type ChannelMaker, type OperationDataChannel } from './peer'

// An operation's channel on a connected peer (`direct-channel.md` § Choosing
// the path): one that closes before it opens while the connection stands,
// as one the runner could not acknowledge on a just reused stream id does,
// is opened once more, and its sibling channels go on; a second such close,
// or one on a connection that went, fails the operation, which then runs on
// the relay. The connection and its channels are played by the test.

/** A data channel the test opens, closes and answers on. */
class PlayedChannel extends EventTarget implements OperationDataChannel {
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
class PlayedConnection implements ChannelMaker {
  connectionState: RTCPeerConnectionState = 'connected'
  readonly channels: PlayedChannel[] = []
  private made = Promise.withResolvers<void>()

  createDataChannel(): PlayedChannel {
    const channel = new PlayedChannel()
    this.channels.push(channel)
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

const answerSchema = z.object({ ok: z.literal(true), size: z.number() })

function probe(path: string): ChannelHeader {
  return { op: 'read', conversation: 'c1', cwd: '/work', path, length: 0 }
}

test('a channel that closes before it opens is opened once more, and its sibling goes on', async () => {
  const connection = new PlayedConnection()
  const logo = openOperation(connection, probe('/work/logo.svg'), answerSchema)
  const picture = openOperation(connection, probe('/work/concept.webp'), answerSchema)
  const [first, sibling] = await Promise.all([connection.channel(0), connection.channel(1)])

  first.end()
  sibling.open()
  const again = await connection.channel(2)
  again.open()
  again.answer(JSON.stringify({ ok: true, size: 14745 }))
  sibling.answer(JSON.stringify({ ok: true, size: 83368 }))

  expect(await (await logo).answer()).toEqual({ ok: true, size: 14745 })
  expect(await (await picture).answer()).toEqual({ ok: true, size: 83368 })
  expect(again.sent.map((text) => JSON.parse(text))).toEqual([probe('/work/logo.svg')])
  expect(first.sent).toEqual([])
  expect(connection.channels).toHaveLength(3)
})

test('a second channel that never opens, or one whose connection went, fails the operation', async () => {
  const connection = new PlayedConnection()
  const twice = openOperation(connection, probe('/work/logo.svg'), answerSchema)
  ;(await connection.channel(0)).end()
  ;(await connection.channel(1)).end()
  await expect(twice).rejects.toBeInstanceOf(ChannelFailed)
  expect(connection.channels).toHaveLength(2)

  const gone = new PlayedConnection()
  const lost = openOperation(gone, probe('/work/logo.svg'), answerSchema)
  gone.connectionState = 'failed'
  ;(await gone.channel(0)).end()
  await expect(lost).rejects.toBeInstanceOf(ChannelFailed)
  expect(gone.channels).toHaveLength(1)
})
