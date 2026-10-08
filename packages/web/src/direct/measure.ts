/**
 * Measuring a device's two paths (`direct-channel.md` § Measuring the
 * paths): one measurement sends 20 probes on each path, 100 ms apart, and
 * counts one not answered within 2 seconds as lost; its figures decide
 * Automatic's choice until the next one.
 */
import type { PathFigures } from '@demicodes/web-ui/devices/direct'

/** The probes a measurement sends on each path. */
export const MEASURE_PROBES = 20
/** The time between a measurement's probes. */
export const PROBE_EVERY_MS = 100
/** How long a probe may take before it counts as lost. */
export const PROBE_LOST_MS = 2000
/** The most of the peer's probes a measurement may lose with the direct path no worse. */
export const WORSE_LOST_PROBES = 1
/** How much slower than the relay the direct path may be. */
export const WORSE_LATENCY_MS = 20

/** One path a measurement probes: how a probe goes, and who hears the answers. */
export interface ProbedPath {
  send(id: number): void
  /** Calls `listener` with the id of each probe answered; the answer stops it. */
  answers(listener: (id: number) => void): () => void
}

/** The page's clock and timers, which a test plays. */
export interface MeasureClock {
  now(): number
  after(ms: number, run: () => void): () => void
}

/** What one measurement found: each path's figures, null for a path not probed or never answered. */
export interface Measured {
  direct: PathFigures | null
  relay: PathFigures | null
}

/** Probe ids grow across measurements, so a late answer of an earlier one is never taken for one of this one's. */
let nextProbeId = 1

/** One path's probes of a measurement. */
class PathRun {
  private readonly sent = new Map<number, number>()
  private readonly rtts: number[] = []
  private readonly stop: () => void

  constructor(
    private readonly path: ProbedPath,
    private readonly clock: MeasureClock,
    /** Whether a lost probe counts: the relay runs over TCP, which loses none. */
    private readonly lossy: boolean,
    answered: () => void,
  ) {
    this.stop = path.answers((id) => {
      const at = this.sent.get(id)
      if (at === undefined) {
        return
      }
      this.sent.delete(id)
      const rtt = this.clock.now() - at
      // An answer after the limit came from a probe already counted as lost.
      if (rtt <= PROBE_LOST_MS) {
        this.rtts.push(rtt)
        answered()
      }
    })
  }

  probe(): void {
    const id = nextProbeId
    nextProbeId = (nextProbeId % 0xffff_ffff) + 1
    this.sent.set(id, this.clock.now())
    this.path.send(id)
  }

  /** Every probe sent came back. */
  get done(): boolean {
    return this.rtts.length === MEASURE_PROBES
  }

  /** Latency, the median round trip of the answered probes, and loss, the share not answered. */
  figures(): PathFigures | null {
    this.stop()
    if (this.rtts.length === 0) {
      return null
    }
    const sorted = [...this.rtts].sort((a, b) => a - b)
    const middle = Math.floor(sorted.length / 2)
    const latencyMs = sorted.length % 2 ? sorted[middle]! : (sorted[middle - 1]! + sorted[middle]!) / 2
    return { latencyMs, loss: this.lossy ? (MEASURE_PROBES - this.rtts.length) / MEASURE_PROBES : null }
  }
}

/**
 * Measures the relay path and, with a peer, the direct one: 20 probes on
 * each, 100 ms apart. It ends once every probe came back, or 2 seconds
 * after the last was sent, and sends nothing after it ended or `signal`
 * aborted it.
 */
export function measurePaths(
  relay: ProbedPath,
  direct: ProbedPath | null,
  clock: MeasureClock,
  signal: AbortSignal,
): Promise<Measured> {
  return new Promise((resolve) => {
    const timers: (() => void)[] = []
    let ended = false
    const finish = () => {
      if (ended) {
        return
      }
      ended = true
      for (const cancel of timers.splice(0)) {
        cancel()
      }
      signal.removeEventListener('abort', finish)
      resolve({ direct: directRun?.figures() ?? null, relay: relayRun.figures() })
    }
    const answered = () => {
      if (relayRun.done && (directRun?.done ?? true)) {
        finish()
      }
    }
    const relayRun = new PathRun(relay, clock, false, answered)
    const directRun = direct ? new PathRun(direct, clock, true, answered) : null
    signal.addEventListener('abort', finish)
    for (let index = 0; index < MEASURE_PROBES; index++) {
      const send = () => {
        relayRun.probe()
        directRun?.probe()
      }
      if (index === 0) {
        send()
      } else {
        timers.push(clock.after(index * PROBE_EVERY_MS, send))
      }
    }
    timers.push(clock.after((MEASURE_PROBES - 1) * PROBE_EVERY_MS + PROBE_LOST_MS, finish))
  })
}

/**
 * Whether the direct path is worse than the relay's: it lost more than one
 * of its 20 probes, or its latency is more than 20 ms above the relay's.
 */
export function directWorse(direct: PathFigures | null, relay: PathFigures | null): boolean {
  if (!direct) {
    return false
  }
  if (Math.round((direct.loss ?? 0) * MEASURE_PROBES) > WORSE_LOST_PROBES) {
    return true
  }
  return relay !== null && direct.latencyMs > relay.latencyMs + WORSE_LATENCY_MS
}
