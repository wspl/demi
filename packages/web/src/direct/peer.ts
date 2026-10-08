/**
 * The page's peer of a direct channel (`direct-channel.md` § Making the
 * channel): an `RTCPeerConnection` with the STUN servers the backend names,
 * whose offer goes at once; its candidates follow as the browser finds them,
 * the public one once a STUN server answers, and the runner's come the
 * other way. Each attempt records what it saw, for the device's page. Each
 * operation opens a data channel of its own, reliable and ordered; its first
 * message is the header, the runner's first the answer (§ Operations on the
 * channel).
 */
import { z } from 'zod'
import { DIRECT_CONNECT_MS, DIRECT_MESSAGE_BYTES, DIRECT_PROBE_LABEL, DIRECT_QUEUE_BYTES } from '@demicodes/protocol'
import type { DirectAddresses, DirectAttempt, DirectPermission, DirectStage } from '@demicodes/web-ui/devices/direct'
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
  /** What the attempt that made the peer saw. */
  readonly attempt: DirectAttempt
  /** Sends probe `id` on the probe channel, unordered and never retransmitted; one that cannot go now is lost. */
  probe(id: number): void
  /** Calls `listener` with each probe the runner sends back; the answer stops it. */
  onProbe(listener: (id: number) => void): () => void
  /** Resolves once the connection failed or closed. */
  readonly closed: Promise<void>
  close(): void
}

/** An attempt that made no peer, with what it saw. */
export class AttemptFailed extends ChannelFailed {
  constructor(
    message: string,
    readonly attempt: DirectAttempt,
  ) {
    super(message)
    this.name = 'AttemptFailed'
  }
}

/** The page's side of a device's signaling, as one attempt uses it. */
export interface PeerSignaling {
  /** Sends the offer and answers the runner's answer; a refusal fails it, `busy` saying so. */
  offer(sdp: string): Promise<string>
  /** Sends a candidate the browser found after the offer. */
  candidate(candidate: string): void
  /** Calls `listener` with each candidate the runner finds after its answer; the answer stops it. */
  candidates(listener: (candidate: string) => void): () => void
}

/** What a peer is made with. */
export interface PeerOptions {
  signaling: PeerSignaling
  /** The STUN servers the backend names; none keeps the peer to one network. */
  stunUrls: readonly string[]
  /** The browser's local network permission as the attempt runs. */
  permission: () => DirectPermission | null
  /** Whether a refusal of the offer is the runner's `busy`. */
  busy: (error: unknown) => boolean
  signal?: AbortSignal
}

/** A candidate line's type and address: `candidate:F 1 udp P <address> <port> typ <type> ...`. */
export function candidateOf(line: string): { type: string; address: string } | null {
  const fields = line.replace(/^a=/, '').split(' ')
  const typed = fields.indexOf('typ')
  const address = fields[4]
  const type = typed > 0 ? fields[typed + 1] : undefined
  if (!line.replace(/^a=/, '').startsWith('candidate:') || !address || !type)
    return null
  return { type, address }
}

/** Adds a candidate's address to the side's addresses it belongs to, each once. */
function note(addresses: DirectAddresses, line: string): void {
  const candidate = candidateOf(line)
  if (!candidate)
    return
  const kind = candidate.type === 'host' ? addresses.local : candidate.type === 'srflx' || candidate.type === 'prflx' ? addresses.public : null
  if (kind && !kind.includes(candidate.address))
    kind.push(candidate.address)
}

/**
 * Makes a peer: its offer goes at once, its candidates follow, and it waits
 * up to the connect limit for the connection, which then stands until it
 * fails or closes. A failure throws `AttemptFailed` with what it saw.
 */
export async function connectPeer(options: PeerOptions): Promise<DirectPeer> {
  const { signaling, signal } = options
  const connection = new RTCPeerConnection({
    iceServers: options.stunUrls.length ? [{ urls: [...options.stunUrls] }] : [],
  })
  const started = Date.now()
  const attempt: DirectAttempt = {
    startedAt: new Date(started).toISOString(),
    durationMs: 0,
    outcome: 'failed',
    stage: null,
    browser: { local: [], public: [] },
    device: { local: [], public: [] },
    pairs: { tried: 0, answered: 0 },
    pair: null,
    permission: options.permission(),
  }
  let iceConnected = false
  const ended = Promise.withResolvers<void>()
  const end = () => {
    stopCandidates()
    connection.close()
    ended.resolve()
  }
  // The runner's candidates may come before its answer: they wait for it.
  let described = false
  const waiting: string[] = []
  const add = (candidate: string) => {
    note(attempt.device, candidate)
    connection.addIceCandidate({ candidate, sdpMLineIndex: 0 }).catch(() => {
      // A candidate the browser cannot use is passed over; the others may connect.
    })
  }
  const stopCandidates = signaling.candidates((candidate) => {
    if (described)
      add(candidate)
    else
      waiting.push(candidate)
  })
  connection.addEventListener('icecandidate', (event: RTCPeerConnectionIceEvent) => {
    const line = event.candidate?.candidate
    if (!line)
      return
    note(attempt.browser, line)
    signaling.candidate(line)
  })
  connection.addEventListener('iceconnectionstatechange', () => {
    if (connection.iceConnectionState === 'connected' || connection.iceConnectionState === 'completed')
      iceConnected = true
  })
  signal?.addEventListener('abort', end, { once: true })
  try {
    // A first channel, so that the offer carries the data channels' section.
    connection.createDataChannel('direct')
    await connection.setLocalDescription(await connection.createOffer())
    const sdp = connection.localDescription?.sdp
    if (!sdp)
      throw new ChannelFailed('The browser made no offer')
    const answer = await options.signaling.offer(sdp)
    for (const line of answer.split(/\r?\n/))
      note(attempt.device, line)
    await connection.setRemoteDescription({ type: 'answer', sdp: answer })
    described = true
    for (const candidate of waiting.splice(0))
      add(candidate)
    await connected(connection)
  } catch (error) {
    attempt.permission = options.permission()
    attempt.durationMs = Date.now() - started
    if (options.busy(error)) {
      attempt.outcome = 'busy'
    } else {
      attempt.stage = failedStage(connection, attempt, iceConnected)
      await countPairs(connection, attempt)
    }
    end()
    throw new AttemptFailed(error instanceof Error ? error.message : String(error), attempt)
  } finally {
    signal?.removeEventListener('abort', end)
  }
  attempt.outcome = 'connected'
  attempt.stage = 'connected'
  attempt.durationMs = Date.now() - started
  await countPairs(connection, attempt)
  connection.addEventListener('connectionstatechange', () => {
    if (connection.connectionState === 'failed' || connection.connectionState === 'closed')
      end()
  })
  // The probe channel measures the direct path (`direct-channel.md`
  // § Measuring the paths): a probe lost stays lost, as a video packet would.
  const probes = connection.createDataChannel(DIRECT_PROBE_LABEL, { ordered: false, maxRetransmits: 0 })
  const probed = new Set<(id: number) => void>()
  probes.addEventListener('message', (event: MessageEvent) => {
    const id = probeId(event.data)
    if (id === null)
      return
    for (const listener of [...probed])
      listener(id)
  })
  return {
    open: (header, answer) => openChannel(connection, header, answer),
    attempt,
    probe: (id) => {
      if (probes.readyState === 'open')
        probes.send(JSON.stringify({ id }))
    },
    onProbe: (listener) => {
      probed.add(listener)
      return () => probed.delete(listener)
    },
    closed: ended.promise,
    close: end,
  }
}

/** The id of a probe the runner sent back; null for anything else. */
function probeId(data: unknown): number | null {
  if (typeof data !== 'string')
    return null
  let value: unknown
  try {
    value = JSON.parse(data)
  } catch {
    // Not a probe: the runner sends back only what the page sent.
    return null
  }
  const parsed = probeSchema.safeParse(value)
  return parsed.success ? parsed.data.id : null
}

const probeSchema = z.object({ id: z.number().int().nonnegative() })

/** Where an attempt that did not connect stopped. */
function failedStage(connection: RTCPeerConnection, attempt: DirectAttempt, iceConnected: boolean): DirectStage {
  if (attempt.permission === 'denied')
    return 'permission'
  if (iceConnected)
    return connection.sctp?.transport.state === 'connected' ? 'channel' : 'handshake'
  if (attempt.browser.local.length === 0 && attempt.browser.public.length === 0)
    return 'gathering'
  return 'checking'
}

/** A report of the browser's statistics, as far as the attempt reads it. */
interface StatsReport {
  type: string
  id: string
  requestsSent?: number
  responsesReceived?: number
  nominated?: boolean
  state?: string
  localCandidateId?: string
  remoteCandidateId?: string
  address?: string
  port?: number
  selectedCandidatePairId?: string
}

/** Counts the address pairs the browser checked and those that answered, and the pair in use. */
async function countPairs(connection: RTCPeerConnection, attempt: DirectAttempt): Promise<void> {
  try {
    const stats = await connection.getStats()
    const reports = new Map<string, StatsReport>()
    stats.forEach((report: StatsReport) => reports.set(report.id, report))
    let tried = 0
    let answered = 0
    let selected: StatsReport | undefined
    for (const report of reports.values()) {
      if (report.type === 'transport' && report.selectedCandidatePairId)
        selected = reports.get(report.selectedCandidatePairId)
      if (report.type !== 'candidate-pair')
        continue
      if ((report.requestsSent ?? 0) > 0)
        tried += 1
      if ((report.responsesReceived ?? 0) > 0)
        answered += 1
      if (!selected && report.nominated && report.state === 'succeeded')
        selected = report
    }
    const local = reports.get(selected?.localCandidateId ?? '')
    const remote = reports.get(selected?.remoteCandidateId ?? '')
    if (remote?.address) {
      // A browser may keep its own address from its statistics, as it hides it behind a name.
      const browser = local?.address ? `${local.address}:${local.port}` : null
      attempt.pair = { browser, device: `${remote.address}:${remote.port}` }
    }
    attempt.pairs = { tried, answered }
  } catch {
    // A closed connection has no statistics left; the counts stay as they were.
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
