/**
 * A device's peer for live views (`direct-channel.md` § Making the
 * channel): the page's second peer with the device, which carries only the
 * live views' `stream` and `video` channels, on an association the runner
 * runs without SCTP's congestion window. It is made when a live view opens
 * on the direct path, shared by every live view on the device, closed a
 * minute after the last one ends, and closed at once when the device's
 * choice leaves `direct`. An attempt still making it ends at once when the
 * last view waiting for it ends, closing its `video` channel with it.
 * Measuring and choosing the path concern the `files` peer alone.
 */
import type { DirectPeer } from './peer'

/** How long the peer stays once its last view ended. */
export const LIVE_IDLE_MS = 60_000

/** What the live peer needs of the page. */
export interface DeviceLiveDeps {
  /** Makes the peer over the device's signaling, as a `live` peer. */
  connect(signal: AbortSignal): Promise<DirectPeer>
  /** Tells the backend the page closed the peer, so that the runner's closes too. */
  closed(): void
  /** Runs `run` after `ms`; the answer cancels it. */
  after(ms: number, run: () => void): () => void
}

/** A live view's use of the peer, which it gives back once when it ends. */
export interface LiveLease {
  /** The connected peer; an attempt that failed rejects. */
  readonly peer: Promise<DirectPeer>
  release(): void
}

/** A peer, connected or being made, with the views that use it. */
interface Held {
  peer: Promise<DirectPeer>
  views: number
}

export class DeviceLive {
  /** The peer; none while no view needs one. */
  private held: Held | null = null
  private attempt: AbortController | null = null
  private cancelIdle: (() => void) | null = null

  constructor(private readonly deps: DeviceLiveDeps) {}

  /** A live view opens on the direct path: it shares the peer, which is made now if there is none. */
  use(): LiveLease {
    this.cancelIdle?.()
    this.cancelIdle = null
    const held = this.held ?? this.start()
    held.views += 1
    let released = false
    return {
      peer: held.peer,
      release: () => {
        if (released)
          return
        released = true
        // A view of a peer that went counts for it alone.
        held.views -= 1
        if (held.views > 0 || this.held !== held)
          return
        // No view waits for a peer still being made: its attempt ends now,
        // rather than holding its opening channel until the connect limit.
        if (this.attempt)
          this.close()
        else
          this.cancelIdle = this.deps.after(LIVE_IDLE_MS, () => {
            this.cancelIdle = null
            this.close()
          })
      },
    }
  }

  /** A channel of `peer` failed, or it went: views that open next make a new one. */
  failed(peer: DirectPeer): void {
    const held = this.held
    void held?.peer.then((current) => {
      if (current === peer && this.held === held)
        this.close()
    }, () => {})
  }

  /** The peer closes now, as when the device's choice left `direct`, and the backend hears so. */
  close(): void {
    if (this.drop())
      this.deps.closed()
  }

  /** The peer is gone already, as the backend or a dropped socket said: it is let go. */
  lost(): void {
    this.drop()
  }

  private start(): Held {
    const attempt = new AbortController()
    this.attempt = attempt
    const held: Held = { peer: this.deps.connect(attempt.signal), views: 0 }
    this.held = held
    held.peer.then(
      (connected) => {
        if (this.attempt === attempt)
          this.attempt = null
        void connected.closed.then(() => {
          if (this.held === held)
            this.drop()
        })
        if (this.held !== held)
          connected.close()
      },
      () => {
        // The views that wait for it take the relay; the next one tries again.
        if (this.held === held) {
          this.held = null
          this.attempt = null
        }
      },
    )
    return held
  }

  /** Lets the peer go, answering whether there was one. */
  private drop(): boolean {
    this.cancelIdle?.()
    this.cancelIdle = null
    const held = this.held
    if (!held)
      return false
    this.held = null
    this.attempt?.abort()
    this.attempt = null
    void held.peer.then((connected) => connected.close(), () => {})
    return true
  }
}
