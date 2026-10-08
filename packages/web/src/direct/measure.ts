/**
 * Measuring a device's two paths (`direct-channel.md` § Measuring the
 * paths): each path's probes, once a second while the page uses the
 * device, their figures over the last 30, and Automatic's choice between
 * the paths, which moves only once a condition held for 10 seconds.
 */
import type { PathFigures } from '@demicodes/web-ui/devices/direct'

/** How long a probe may take before it counts as lost. */
export const PROBE_LOST_MS = 2000
/** The probes the figures are computed over. */
export const FIGURE_PROBES = 30
/** How long a condition must hold before Automatic moves, and the window it judges. */
export const HOLD_MS = 10_000
/** The loss above which the direct path is worse. */
export const WORSE_LOSS = 0.02
/** How much slower than the relay the direct path may be. */
export const WORSE_LATENCY_MS = 20

/** One probe's outcome: its round trip, or lost. */
interface Sample {
  at: number
  rtt: number | null
}

/** One path's probes and what came of them. */
export class PathProbes {
  private nextId = 1
  private readonly waiting = new Map<number, number>()
  private samples: Sample[] = []

  /** Whether a lost probe counts: the relay runs over TCP, which loses none. */
  constructor(private readonly lossy: boolean) {}

  /** Records a probe sent at `now`; answers its id. */
  send(now: number): number {
    const id = this.nextId
    this.nextId = (this.nextId % 0xffff_ffff) + 1
    this.waiting.set(id, now)
    return id
  }

  /** Probe `id` came back at `now`; one already lost or unknown is passed over. */
  answered(id: number, now: number): void {
    const sent = this.waiting.get(id)
    if (sent === undefined) {
      return
    }
    this.waiting.delete(id)
    this.add({ at: sent, rtt: now - sent })
  }

  /** Probes unanswered for the loss limit by `now` count as lost. */
  expire(now: number): void {
    for (const [id, sent] of this.waiting) {
      if (now - sent >= PROBE_LOST_MS) {
        this.waiting.delete(id)
        this.add({ at: sent, rtt: null })
      }
    }
  }

  /** Forgets every probe, as a new peer's path is another. */
  clear(): void {
    this.waiting.clear()
    this.samples = []
  }

  /** The figures over the last 30 probes; null before one was answered. */
  figures(): PathFigures | null {
    return figuresOf(this.samples.slice(-FIGURE_PROBES), this.lossy)
  }

  /** The figures over the probes sent in the 10 seconds before `now`. */
  recent(now: number): PathFigures | null {
    return figuresOf(this.samples.filter((sample) => now - sample.at <= HOLD_MS), this.lossy)
  }

  private add(sample: Sample): void {
    this.samples.push(sample)
    this.samples.sort((a, b) => a.at - b.at)
    if (this.samples.length > FIGURE_PROBES * 2) {
      this.samples.splice(0, this.samples.length - FIGURE_PROBES * 2)
    }
  }
}

/** Latency, the median round trip, and loss, the share lost. */
function figuresOf(samples: readonly Sample[], lossy: boolean): PathFigures | null {
  const rtts = samples.flatMap((sample) => (sample.rtt === null ? [] : [sample.rtt]))
  if (rtts.length === 0) {
    return null
  }
  const sorted = [...rtts].sort((a, b) => a - b)
  const middle = Math.floor(sorted.length / 2)
  const latencyMs = sorted.length % 2 ? sorted[middle]! : (sorted[middle - 1]! + sorted[middle]!) / 2
  const loss = lossy ? (samples.length - rtts.length) / samples.length : null
  return { latencyMs, loss }
}

/** Whether the direct path is worse than the relay's: losing over 2%, or over 20 ms slower. */
export function directWorse(direct: PathFigures | null, relay: PathFigures | null): boolean {
  if (!direct) {
    return false
  }
  if ((direct.loss ?? 0) > WORSE_LOSS) {
    return true
  }
  return relay !== null && direct.latencyMs > relay.latencyMs + WORSE_LATENCY_MS
}

/**
 * Automatic's choice between a connected peer and the relay: the peer, at
 * once when it connects, until the direct path has been worse for 10
 * seconds; then the relay, until it has been no worse for 10 seconds. One
 * slow probe never moves an open live view.
 */
export class AutomaticChoice {
  private slower = false
  /** Since when the other path's condition holds; null while it does not. */
  private since: number | null = null

  /** A new peer is used at once, before its figures exist. */
  reset(): void {
    this.slower = false
    this.since = null
  }

  /** Whether the direct path is the slower one at `now`, given whether it is worse now. */
  update(now: number, worse: boolean): boolean {
    if (worse === this.slower) {
      this.since = null
      return this.slower
    }
    this.since ??= now
    if (now - this.since >= HOLD_MS) {
      this.slower = worse
      this.since = null
    }
    return this.slower
  }
}
