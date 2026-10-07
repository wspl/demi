/**
 * The page's peer of a direct channel (`direct-channel.md` § Making the
 * channel): an `RTCPeerConnection` with no ICE servers, whose offer goes at
 * once, before its own candidates, since the runner learns the page's
 * addresses from the checks the page sends it. Each operation opens a data
 * channel of its own, reliable and ordered; its first message is the
 * header, the runner's first the answer (§ Operations on the channel).
 */
import type { z } from 'zod'
import { DIRECT_CONNECT_MS, DIRECT_MESSAGE_BYTES, DIRECT_QUEUE_BYTES } from '@demicodes/protocol'
import { channelRefusalSchema, type ChannelErrorCode, type ChannelHeader } from '../api/generated/web-api'

/**
 * The channel or the peer failed under an operation: the operation runs
 * again on the relay, or fails there.
 */
export class ChannelFailed extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'ChannelFailed'
  }
}

/** The runner refused the operation, with the code and status the relay's route answers. */
export class ChannelRefused extends Error {
  constructor(
    readonly code: ChannelErrorCode,
    readonly status: number,
    message: string,
  ) {
    super(message)
    this.name = 'ChannelRefused'
  }
}

/**
 * Whether a refusal sends the operation to the relay rather than answering
 * it: a peer at its limit of channels, or an answer too large for one
 * message on the channel.
 */
export function relayAnswers(error: unknown): boolean {
  return error instanceof ChannelFailed
    || (error instanceof ChannelRefused && (error.code === 'busy' || error.code === 'too_large'))
}

/** A message of the runner's after its answer. */
export type ChannelMessage = { kind: 'text'; text: string } | { kind: 'bytes'; bytes: Uint8Array }

/** One operation's open channel, its header sent. */
export interface OperationChannel<T> {
  /**
   * The runner's answer, its first message: at once for most operations,
   * once its file is in place for a write. A refusal throws `ChannelRefused`.
   */
  answer(): Promise<T>
  /** The runner's next message after its answer; null once it closed the channel, its end. */
  next(): Promise<ChannelMessage | null>
  /** Sends text, once the channel's queue has room. */
  sendText(text: string): Promise<void>
  /** Sends bytes in messages of at most the channel's size, each once the queue has room. */
  sendBytes(bytes: Uint8Array): Promise<void>
  close(): void
}

/** A connected peer. */
export interface DirectPeer {
  /** Opens an operation's channel and sends its header; the runner's answer is checked against `answer`. */
  open<T>(header: ChannelHeader, answer: z.ZodType<T>): Promise<OperationChannel<T>>
  /** Resolves once the connection failed or closed. */
  readonly closed: Promise<void>
  close(): void
}

/** Sends an offer to the runner through the backend and answers its answer. */
export type Offer = (sdp: string) => Promise<string>

/**
 * Makes a peer: its offer goes through `offer` at once, and it waits up to
 * the connect limit for the connection, which then stands until it fails
 * or closes.
 */
export async function connectPeer(offer: Offer, signal?: AbortSignal): Promise<DirectPeer> {
  const connection = new RTCPeerConnection({ iceServers: [] })
  const ended = Promise.withResolvers<void>()
  const end = () => {
    connection.close()
    ended.resolve()
  }
  signal?.addEventListener('abort', end, { once: true })
  try {
    // A first channel, so that the offer carries the data channels' section.
    connection.createDataChannel('direct')
    await connection.setLocalDescription(await connection.createOffer())
    const sdp = connection.localDescription?.sdp
    if (!sdp)
      throw new ChannelFailed('The browser made no offer')
    const answer = await offer(sdp)
    await connection.setRemoteDescription({ type: 'answer', sdp: answer })
    await connected(connection)
  } catch (error) {
    end()
    throw error
  } finally {
    signal?.removeEventListener('abort', end)
  }
  connection.addEventListener('connectionstatechange', () => {
    if (connection.connectionState === 'failed' || connection.connectionState === 'closed')
      end()
  })
  return {
    open: (header, answer) => openChannel(connection, header, answer),
    closed: ended.promise,
    close: end,
  }
}

/** Resolves once `connection` connects, and fails when it fails or the connect limit passes first. */
function connected(connection: RTCPeerConnection): Promise<void> {
  return new Promise((resolve, reject) => {
    const settle = (error: Error | null) => {
      clearTimeout(timer)
      connection.removeEventListener('connectionstatechange', change)
      if (error)
        reject(error)
      else
        resolve()
    }
    const change = () => {
      const state = connection.connectionState
      if (state === 'connected')
        settle(null)
      else if (state === 'failed' || state === 'closed')
        settle(new ChannelFailed(`The direct connection ${state}`))
    }
    const timer = setTimeout(() => settle(new ChannelFailed('The direct connection did not connect in time')), DIRECT_CONNECT_MS)
    connection.addEventListener('connectionstatechange', change)
    change()
  })
}

/** A queue of a channel's messages, which its reader takes one at a time. */
class Inbox {
  private readonly messages: ChannelMessage[] = []
  private waiting: PromiseWithResolvers<ChannelMessage | null> | null = null
  private end: ChannelFailed | 'closed' | null = null

  push(message: ChannelMessage): void {
    if (this.waiting) {
      this.waiting.resolve(message)
      this.waiting = null
      return
    }
    this.messages.push(message)
  }

  /** The channel ended: cleanly, with what it sent, or failed. */
  finish(end: ChannelFailed | 'closed'): void {
    this.end ??= end
    if (this.waiting) {
      if (end === 'closed')
        this.waiting.resolve(null)
      else
        this.waiting.reject(end)
      this.waiting = null
    }
  }

  next(): Promise<ChannelMessage | null> {
    const message = this.messages.shift()
    if (message)
      return Promise.resolve(message)
    if (this.end === 'closed')
      return Promise.resolve(null)
    if (this.end)
      return Promise.reject(this.end)
    this.waiting = Promise.withResolvers()
    return this.waiting.promise
  }
}

async function openChannel<T>(
  connection: RTCPeerConnection,
  header: ChannelHeader,
  answerSchema: z.ZodType<T>,
): Promise<OperationChannel<T>> {
  let channel: RTCDataChannel
  try {
    channel = connection.createDataChannel(header.op, { ordered: true })
  } catch (error) {
    throw new ChannelFailed(error instanceof Error ? error.message : String(error))
  }
  channel.binaryType = 'arraybuffer'
  channel.bufferedAmountLowThreshold = DIRECT_QUEUE_BYTES / 2
  const inbox = new Inbox()
  channel.addEventListener('open', () => channel.send(JSON.stringify(header)))
  channel.addEventListener('message', (event: MessageEvent) => {
    if (typeof event.data === 'string')
      inbox.push({ kind: 'text', text: event.data })
    else if (event.data instanceof ArrayBuffer)
      inbox.push({ kind: 'bytes', bytes: new Uint8Array(event.data) })
  })
  // A channel closes as its operation's end; one that closes because its
  // connection went is a failure.
  channel.addEventListener('close', () => {
    const lost = connection.connectionState !== 'connected'
    inbox.finish(lost ? new ChannelFailed('The direct connection went') : 'closed')
  })
  channel.addEventListener('error', () => inbox.finish(new ChannelFailed('The direct channel failed')))
  await new Promise<void>((resolve, reject) => {
    channel.addEventListener('open', () => resolve(), { once: true })
    channel.addEventListener('close', () => reject(new ChannelFailed('The direct channel did not open')), { once: true })
  })
  let answered: Promise<T> | null = null
  const answer = () => {
    answered ??= inbox.next().then((first) => {
      if (first === null || first.kind !== 'text')
        throw new ChannelFailed('The direct channel ended before its answer')
      return parseAnswer(first.text, answerSchema)
    })
    return answered
  }
  const room = async () => {
    while (channel.bufferedAmount > DIRECT_QUEUE_BYTES) {
      if (channel.readyState !== 'open')
        throw new ChannelFailed('The direct channel closed')
      await new Promise((resolve) => {
        channel.addEventListener('bufferedamountlow', resolve, { once: true })
        channel.addEventListener('close', resolve, { once: true })
      })
    }
    if (channel.readyState !== 'open')
      throw new ChannelFailed('The direct channel closed')
  }
  return {
    answer,
    async next() {
      await answer()
      return inbox.next()
    },
    async sendText(text) {
      await room()
      channel.send(text)
    },
    async sendBytes(bytes) {
      for (let offset = 0; offset < bytes.length; offset += DIRECT_MESSAGE_BYTES) {
        await room()
        channel.send(bytes.slice(offset, offset + DIRECT_MESSAGE_BYTES))
      }
    },
    close: () => channel.close(),
  }
}

/** The runner's first message: its refusal, or the operation's answer as `schema` reads it. */
function parseAnswer<T>(text: string, schema: z.ZodType<T>): T {
  let value: unknown
  try {
    value = JSON.parse(text)
  } catch {
    throw new ChannelFailed('The runner answered with no JSON')
  }
  const refusal = channelRefusalSchema.safeParse(value)
  if (refusal.success) {
    const { code, status, message } = refusal.data.error
    throw new ChannelRefused(code, status, message)
  }
  const answer = schema.safeParse(value)
  if (!answer.success)
    throw new ChannelFailed(`The runner’s answer does not read: ${answer.error.issues[0]?.message ?? 'unknown shape'}`)
  return answer.data
}
