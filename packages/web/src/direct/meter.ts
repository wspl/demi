/**
 * The measuring of one device's paths (`direct-channel.md` § Measuring the
 * paths): a measurement only when something reads it, when a peer connects,
 * the device's page shows or the user selects Measure, never on a timer.
 * Its figures are the device page's until the next one replaces them, and
 * Automatic decides from them.
 */
import type { DeviceRoute, PathFigures } from '@demicodes/web-ui/devices/direct'
import { directWorse, measurePaths, type MeasureClock } from './measure'
import type { DirectPeer } from './peer'
import type { DeviceSignaling } from './signaling'

/** What the views show of a device's measured paths. */
export interface MeterState {
  /** Each path's figures from the last measurement; null before one found any. */
  figures: { direct: PathFigures | null; relay: PathFigures | null }
  /** A measurement runs. */
  measuring: boolean
}

export function meterState(): MeterState {
  return { figures: { direct: null, relay: null }, measuring: false }
}

/** The page's own clock. */
export const pageClock: MeasureClock = {
  now: () => performance.now(),
  after: (ms, run) => {
    const timer = setTimeout(run, ms)
    return () => clearTimeout(timer)
  },
}

export class DeviceMeter {
  /** The measurement that runs and the peer it probes, which another one asked for meanwhile joins. */
  private running: { peer: Pick<DirectPeer, 'probe' | 'onProbe'> | null; done: Promise<void> } | null = null
  private readonly closed = new AbortController()

  constructor(
    /** The device's choice of path: its peer to probe, and what Automatic found. */
    private readonly choice: {
      connected(): Pick<DirectPeer, 'probe' | 'onProbe'> | null
      setSlower(slower: boolean): void
    },
    private readonly signaling: Pick<DeviceSignaling, 'ping' | 'onPong'>,
    /** The device's route as every page of the user's sees it. */
    private readonly route: () => DeviceRoute,
    /** Where the figures are kept for the views; a reactive object in the page. */
    readonly state: MeterState = meterState(),
    private readonly clock: MeasureClock = pageClock,
  ) {}

  /**
   * Measures both paths, or joins the measurement that runs; it ends within
   * 4 seconds. One that runs without the peer now connected, which it
   * started before, is followed by one with it.
   */
  measure(): Promise<void> {
    if (this.closed.signal.aborted) {
      return Promise.resolve()
    }
    const peer = this.choice.connected()
    if (this.running) {
      return this.running.peer === peer ? this.running.done : this.running.done.then(() => this.measure())
    }
    const done = this.run(peer).finally(() => {
      this.running = null
      this.state.measuring = false
    })
    this.running = { peer, done }
    return done
  }

  /** Ends the measurement that runs, as the device's use ends. */
  close(): void {
    this.closed.abort()
  }

  private async run(peer: Pick<DirectPeer, 'probe' | 'onProbe'> | null): Promise<void> {
    this.state.measuring = true
    const measured = await measurePaths(
      { send: (id) => this.signaling.ping(id), answers: (listener) => this.signaling.onPong(listener) },
      peer ? { send: (id) => peer.probe(id), answers: (listener) => peer.onProbe(listener) } : null,
      this.clock,
      this.closed.signal,
    )
    if (this.closed.signal.aborted) {
      return
    }
    this.state.figures = { direct: peer ? measured.direct : null, relay: measured.relay }
    // A peer that went or was replaced meanwhile has its own measurement.
    if (peer && peer === this.choice.connected() && this.route() === 'automatic') {
      this.choice.setSlower(directWorse(measured.direct, measured.relay))
    }
  }
}
