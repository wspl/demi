/**
 * The measuring of one device's paths while the page uses it
 * (`direct-channel.md` § Measuring the paths): a probe on each path once a
 * second, their figures for the device's page, Automatic's finding for the
 * choice of path, and Test Speed when the user asks.
 */
import { z } from 'zod'
import type { DeviceRoute, PathFigures, SpeedResult } from '@demicodes/web-ui/devices/direct'
import { apiRequest } from '../api/client'
import type { DeviceDirect } from './device'
import { AutomaticChoice, PathProbes, directWorse } from './measure'
import type { DirectPeer } from './peer'
import type { DeviceSignaling } from './signaling'

/** How often each path is probed. */
const PROBE_EVERY_MS = 1000
/** What one speed test downloads over each path: 8 MiB. */
export const SPEED_BYTES = 8 * 1024 * 1024
const MIB = 1024 * 1024

/** What the views show of a device's measured paths. */
export interface MeterState {
  figures: { direct: PathFigures | null; relay: PathFigures | null }
  speed: SpeedResult
}

export function meterState(): MeterState {
  return { figures: { direct: null, relay: null }, speed: { direct: null, relay: null, testing: false } }
}

export class DeviceMeter {
  private readonly direct = new PathProbes(true)
  private readonly relay = new PathProbes(false)
  private readonly automatic = new AutomaticChoice()
  /** The peer whose probe channel is followed, and the end of following it. */
  private probed: { peer: DirectPeer; stop: () => void } | null = null
  private stopPongs: (() => void) | null = null
  private timer: ReturnType<typeof setInterval> | null = null
  /** How many uses of the device measure it: its page, the conversation shown. */
  private users = 0

  constructor(
    private readonly deviceId: string,
    private readonly choice: DeviceDirect,
    private readonly signaling: DeviceSignaling,
    /** The device's route as every page of the user's sees it. */
    private readonly route: () => DeviceRoute,
    /** Where the figures are kept for the views; a reactive object in the page. */
    readonly state: MeterState = meterState(),
  ) {}

  /** A use of the device begins: it is measured until each use ended. The answer ends this use. */
  use(): () => void {
    this.users += 1
    if (this.users === 1)
      this.start()
    let ended = false
    return () => {
      if (ended)
        return
      ended = true
      this.users -= 1
      if (this.users === 0)
        this.stop()
    }
  }

  /** Ends every measuring, as the device's use ends. */
  close(): void {
    this.users = 0
    this.stop()
  }

  /**
   * Test Speed: 8 MiB over each path in turn, direct first when there is a
   * peer, as MiB/s. One test runs at a time.
   */
  async testSpeed(): Promise<void> {
    const speed = this.state.speed
    if (speed.testing)
      return
    speed.testing = true
    try {
      const peer = this.choice.connected()
      speed.direct = peer ? await timed(() => directBytes(peer)) : null
      speed.relay = await timed(() => relayBytes(this.deviceId))
    } finally {
      speed.testing = false
    }
  }

  private start(): void {
    this.stopPongs = this.signaling.onPong((id) => this.relay.answered(id, performance.now()))
    this.timer = setInterval(() => this.tick(), PROBE_EVERY_MS)
    this.tick()
  }

  private stop(): void {
    if (this.timer !== null)
      clearInterval(this.timer)
    this.timer = null
    this.stopPongs?.()
    this.stopPongs = null
    this.probed?.stop()
    this.probed = null
  }

  /** One second of measuring: a probe on each path, then what they show. */
  private tick(): void {
    const now = performance.now()
    this.signaling.ping(this.relay.send(now))
    const peer = this.choice.connected()
    if (peer !== this.probed?.peer) {
      // Another peer's path is another: its figures start anew.
      this.probed?.stop()
      this.direct.clear()
      this.automatic.reset()
      this.probed = peer ? { peer, stop: peer.onProbe((id) => this.direct.answered(id, performance.now())) } : null
    }
    if (peer)
      peer.probe(this.direct.send(now))
    this.direct.expire(now)
    this.relay.expire(now)
    this.state.figures = { direct: peer ? this.direct.figures() : null, relay: this.relay.figures() }
    if (peer && this.route() === 'automatic')
      this.choice.setSlower(this.automatic.update(now, directWorse(this.direct.recent(now), this.relay.recent(now))))
  }
}

/** The MiB/s of the bytes `read` reads; null when it fails. */
async function timed(read: () => Promise<number>): Promise<number | null> {
  const started = performance.now()
  try {
    const bytes = await read()
    const seconds = (performance.now() - started) / 1000
    return seconds > 0 ? bytes / MIB / seconds : null
  } catch {
    // A path that failed under the test has no speed to show; the page says so with a dash.
    return null
  }
}

const speedAnswerSchema = z.object({ ok: z.literal(true) })

/** Downloads a speed test's bytes over the peer's `speed` channel; answers how many came. */
async function directBytes(peer: DirectPeer): Promise<number> {
  const channel = await peer.open({ op: 'speed', bytes: SPEED_BYTES }, speedAnswerSchema)
  await channel.answer()
  let bytes = 0
  for (;;) {
    const message = await channel.next()
    if (message === null)
      return bytes
    if (message.kind === 'bytes')
      bytes += message.bytes.length
  }
}

/** Downloads a speed test's bytes through the server; answers how many came. */
async function relayBytes(deviceId: string): Promise<number> {
  const response = await apiRequest(`/devices/${encodeURIComponent(deviceId)}/speed?bytes=${SPEED_BYTES}`, { waits: false })
  if (!response.ok || !response.body)
    throw new Error(`The speed route answered ${response.status}`)
  let bytes = 0
  const reader = response.body.getReader()
  for (;;) {
    const { done, value } = await reader.read()
    if (done)
      return bytes
    bytes += value.length
  }
}
