/**
 * The page's choice of path to one paired device (`direct-channel.md`
 * § Choosing the path): `direct` while its peer is connected and the
 * device's route takes it, under Automatic only while the direct path is
 * not the slower one, `relay` otherwise. Every operation starts on the relay at once and chooses as it
 * starts; nothing waits for the direct channel. While the choice is `relay`
 * the page tries again: at once for each reason it is given (the browser's
 * local network permission changed, the page came back online, the
 * device's runner or the signaling socket connected again), and otherwise
 * after 1, 2 and 5 minutes and then every 10. One attempt runs at a time.
 * A browser that blocks local network access gets no attempt until the
 * permission changes, and a device whose route is Server Only gets none, and
 * loses its peer, until the route allows one again. Each attempt leaves what
 * it saw for the device's page.
 */
import type { DeviceRoute, DirectAttempt } from '@demicodes/web-ui/devices/direct'
import { AttemptFailed, type DirectPeer } from './peer'

export type Choice = 'relay' | 'direct'

/** The browser's local network permission, where it reports one. */
export type Permission = 'granted' | 'prompt' | 'denied'

/** The waits between attempts that failed in a row; the last repeats. */
export const RETRY_MS = [60_000, 120_000, 300_000, 600_000] as const

/** What a device's choice needs of the page. */
export interface DeviceDirectDeps {
  /**
   * Whether an attempt can start: the device's signaling socket is open.
   * Its opening is a reason to try, so nothing waits for it.
   */
  ready(): boolean
  /** Makes a peer through the device's signaling; fails when it does not connect. */
  connect(signal: AbortSignal): Promise<DirectPeer>
  /** Calls `run` after `ms`, unless the answer is called first. */
  after(ms: number, run: () => void): () => void
}

/** What the page shows of a device's path. */
export interface DirectState {
  choice: Choice
  /** A peer is connected, whether or not the choice uses it. */
  peer: boolean
  /** The browser blocks local network access. */
  blocked: boolean
  /** An attempt runs now. */
  trying: boolean
  /** What the last attempt saw; null before the first. */
  attempt: DirectAttempt | null
  /** When the next attempt runs, in milliseconds since the epoch; null when none is planned. */
  nextAt: number | null
}

/** The state of a device the page has not tried yet. */
export function directState(): DirectState {
  return { choice: 'relay', peer: false, blocked: false, trying: false, attempt: null, nextAt: null }
}

export class DeviceDirect {
  private peer: DirectPeer | null = null
  private attempt: AbortController | null = null
  private cancelRetry: (() => void) | null = null
  /** Attempts that failed in a row since the last connected or a reason to try came. */
  private failures = 0
  private permission: Permission | null = null
  private stopped = false
  /** The device's route. */
  private route: DeviceRoute = 'automatic'
  /** Automatic found the direct path the slower one (`measure.ts`). */
  private slower = false
  private readonly listeners = new Set<(choice: Choice) => void>()

  constructor(
    private readonly deps: DeviceDirectDeps,
    /** Where the choice is kept for the views; a reactive object in the page. */
    readonly state: DirectState = directState(),
  ) {}

  get choice(): Choice {
    return this.state.choice
  }

  /** The connected peer, while the choice is `direct`. */
  current(): DirectPeer | null {
    return this.state.choice === 'direct' ? this.peer : null
  }

  /** The connected peer, whether or not the choice uses it, as its path is measured. */
  connected(): DirectPeer | null {
    return this.peer
  }

  /** Calls `listener` with each new choice; the answer stops it. */
  onChange(listener: (choice: Choice) => void): () => void {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  /**
   * A reason to try again came: an attempt starts now, unless the choice
   * is `direct`, one runs, or the browser blocks local network access.
   */
  tryNow(): void {
    if (this.stopped || this.route === 'server' || this.peer || this.attempt || this.permission === 'denied' || !this.deps.ready())
      return
    this.cancelRetry?.()
    this.cancelRetry = null
    this.state.nextAt = null
    this.failures = 0
    this.start()
  }

  /**
   * The device's route: Server Only stops the attempt, closes the peer and
   * tries nothing more; a route that allows a peer again tries at once; and
   * Prefer Direct uses a connected peer whatever Automatic found.
   */
  setRoute(route: DeviceRoute): void {
    if (route === this.route)
      return
    const was = this.route
    this.route = route
    if (route !== 'server') {
      this.choose()
      if (was === 'server')
        this.tryNow()
      return
    }
    this.cancelRetry?.()
    this.cancelRetry = null
    this.state.nextAt = null
    this.attempt?.abort()
    this.attempt = null
    this.state.trying = false
    const peer = this.peer
    if (peer) {
      this.peer = null
      this.state.peer = false
      this.choose()
      peer.close()
    }
  }

  /** What Automatic found of the paths: whether the direct one is the slower one now. */
  setSlower(slower: boolean): void {
    this.slower = slower
    this.choose()
  }

  /** The browser reported its local network permission; a change is a reason to try again. */
  setPermission(permission: Permission | null): void {
    const changed = permission !== this.permission
    this.permission = permission
    this.state.blocked = permission === 'denied'
    if (permission === 'denied') {
      this.cancelRetry?.()
      this.cancelRetry = null
      this.state.nextAt = null
      return
    }
    if (changed)
      this.tryNow()
  }

  /**
   * A channel of the peer failed: the choice is `relay` at once, and the
   * page tries again later.
   */
  failed(): void {
    const peer = this.peer
    if (!peer)
      return
    this.lost(peer)
    peer.close()
  }

  /**
   * The backend closed the peer, since the user turned a plugin on or off:
   * the choice is `relay`, and an attempt with a new introduction starts at
   * once, in place of one that runs with the old.
   */
  reintroduce(): void {
    this.attempt?.abort()
    this.attempt = null
    const peer = this.peer
    if (peer) {
      this.lost(peer)
      peer.close()
    }
    this.tryNow()
  }

  /** The page no longer uses the device: its peer closes, and nothing tries again. */
  stop(): void {
    this.stopped = true
    this.cancelRetry?.()
    this.cancelRetry = null
    this.attempt?.abort()
    this.attempt = null
    const peer = this.peer
    if (peer) {
      this.lost(peer)
      peer.close()
    }
    this.listeners.clear()
  }

  private start(): void {
    const attempt = new AbortController()
    this.attempt = attempt
    this.state.trying = true
    this.deps.connect(attempt.signal).then(
      (peer) => {
        if (this.attempt !== attempt) {
          peer.close()
          return
        }
        this.attempt = null
        this.state.trying = false
        this.state.attempt = peer.attempt
        this.failures = 0
        this.peer = peer
        this.state.peer = true
        // A new peer is used at once, before its figures exist.
        this.slower = false
        this.choose()
        void peer.closed.then(() => this.lost(peer))
      },
      (error: unknown) => {
        if (this.attempt !== attempt)
          return
        this.attempt = null
        this.state.trying = false
        if (error instanceof AttemptFailed)
          this.state.attempt = error.attempt
        this.failures += 1
        this.schedule()
      },
    )
  }

  /** The peer went: the choice is `relay`, and the page tries again later. */
  private lost(peer: DirectPeer): void {
    if (this.peer !== peer)
      return
    this.peer = null
    this.state.peer = false
    this.state.attempt = { ...peer.attempt, outcome: 'dropped', endedAt: new Date().toISOString() }
    this.choose()
    this.failures += 1
    this.schedule()
  }

  private schedule(): void {
    if (this.stopped || this.route === 'server' || this.permission === 'denied')
      return
    this.cancelRetry?.()
    const wait = RETRY_MS[Math.min(this.failures, RETRY_MS.length) - 1] ?? RETRY_MS[0]
    this.state.nextAt = Date.now() + wait
    this.cancelRetry = this.deps.after(wait, () => {
      this.cancelRetry = null
      this.state.nextAt = null
      if (!this.peer && !this.attempt && !this.stopped && this.route !== 'server' && this.deps.ready())
        this.start()
    })
  }

  /** The choice the peer, the route and Automatic's finding make. */
  private choose(): void {
    const usable = this.peer !== null && (this.route === 'direct' || !this.slower)
    this.set(usable ? 'direct' : 'relay')
  }

  private set(choice: Choice): void {
    if (this.state.choice === choice)
      return
    this.state.choice = choice
    for (const listener of [...this.listeners])
      listener(choice)
  }
}
