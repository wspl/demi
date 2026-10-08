/**
 * The measuring of one device's paths while the page uses it
 * (`direct-channel.md` § Measuring the paths): a probe on each path once a
 * second, their figures for the device's page, Automatic's finding for the
 * choice of path.
 */
import type { DeviceRoute, PathFigures } from '@demicodes/web-ui/devices/direct'
import type { DeviceDirect } from './device'
import { AutomaticChoice, PathProbes, directWorse } from './measure'
import type { DirectPeer } from './peer'
import type { DeviceSignaling } from './signaling'

/** How often each path is probed. */
const PROBE_EVERY_MS = 1000

/** What the views show of a device's measured paths. */
export interface MeterState {
  figures: { direct: PathFigures | null; relay: PathFigures | null }
}

export function meterState(): MeterState {
  return { figures: { direct: null, relay: null } }
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
