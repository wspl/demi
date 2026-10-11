/**
 * A file watch that carries the subscriptions of what a page keeps of one
 * Host's files (`web-api.md` § File watch, `plugin-pages.md` § What the
 * service keeps): a conversation's watch, on the relay or its direct
 * channel, or a device's folder dialog's. The kept files (`HostFiles`)
 * hand each subscription to the watch of the view that brought it; the
 * watch is open while it carries any, subscribes again to all of them each
 * time it opens, renews them in one message every 3 minutes, and tells the
 * kept files what the Host says of each.
 *
 * Nothing here decides whether an entry is current: a subscription holds
 * what the Host said, its `since`, the `at` of its last change and when its
 * lease began as the page counts it, and the kept files compare those with
 * each answer's number.
 */
import { shallowReactive } from 'vue'
import { waitToReconnect, watchSilence, type ReconnectWait, type SilenceWatch } from '../transport/liveness'

/** How long a lease runs; the page counts it from when it sent the message the Host's answer acknowledges. */
export const LEASE_MS = 10 * 60 * 1000

/** How often the page renews every subscription it holds, in one message. */
export const RENEW_MS = 3 * 60 * 1000

/** The most subscriptions or keys one message names (`web-api.md` § File watch). */
const MESSAGE_KEYS = 4096

/** What a subscription watches, and so when it changes (`runner.md` § Watching files). */
export type SubscriptionKind = 'file' | 'folder_names' | 'folder_details' | 'worktree' | 'head'

/** What a subscription is to: a kind and an absolute path on the Host. */
export interface Subject {
  kind: SubscriptionKind
  path: string
}

/** Where in the Host's run of numbers a read began; another epoch's numbers compare with nothing. */
export interface ReadAt {
  epoch: string
  number: number
}

/** A message the page sends on a watch. */
export type WatchRequest =
  | { type: 'subscribe'; subscriptions: { key: string; kind: SubscriptionKind; path: string }[] }
  | { type: 'unsubscribe'; keys: string[] }
  | { type: 'renew'; keys: string[] }

/** A message a watch brings, read against its schema where it entered the page. */
export type WatchMessage =
  | { type: 'live'; key: string; since: number; epoch: string }
  | { type: 'changed'; changes: [string, number][] }
  | { type: 'unavailable'; key: string; reason: string }
  | { type: 'refused'; key: string; reason: string }
  | { type: 'renewed'; keys: string[] }
  | { type: 'offline' }
  | { type: 'heartbeat' }

/** What one connection of a watch tells the watch. */
export interface WatchLinkHandlers {
  /** It opened: what it says from now on counts. */
  opened(): void
  message(message: WatchMessage): void
  /** It closed, failed, or brought what did not read. */
  closed(): void
}

/** One connection of a watch: a WebSocket of the relay's or a direct channel's `watch`. */
export interface WatchLink {
  send(request: WatchRequest): void
  close(): void
}

/** Opens one connection of a watch. */
export type OpenWatchLink = (handlers: WatchLinkHandlers) => WatchLink

/**
 * Where a subscription stands: subscribed and not yet answered, or after
 * its watch closed (`waiting`); watched (`live`); or over, the Host unable
 * to watch it or the page past its limits, until the watch opens again.
 */
export type SubscriptionStatus = 'waiting' | 'live' | 'unavailable' | 'refused'

/** What the page says when the Host refuses a subscription for the page's limits. */
const REFUSED_REASON = 'This page watches too many files.'

/** One subscription of the kept files, shared by every entry that depends on its subject. */
export class Subscription {
  /** What the views read of it. */
  readonly state = shallowReactive({ status: 'waiting' as SubscriptionStatus, reason: null as string | null })
  /** The epoch of the run that watches it, from its last `live`. */
  epoch: string | null = null
  /** The number its watch started at. */
  since = 0
  /** How many times it went live, which tells a read whether a watch of another run started meanwhile. */
  lives = 0
  /** The number of its latest change in `epoch`'s run; -1 for none. */
  changedAt = -1
  /** When the page sent the message its lease runs from; null while no answer acknowledged one. */
  leaseFrom: number | null = null
  /** When the page last sent its `subscribe`. */
  subscribedAt = 0
  /** How many kept entries depend on it. */
  holders = 0
  /** The watch that carries it; none for a source without one. */
  channel: WatchChannel | null = null

  constructor(readonly key: string, readonly subject: Subject) {}

  /** Whether its lease is over as the page counts it at `now`. */
  lapsed(now: number): boolean {
    return (this.leaseFrom ?? this.subscribedAt) + LEASE_MS <= now
  }

  /** Whether the Host watches it, as the page counts the lease at `now`. */
  watching(now: number): boolean {
    return this.state.status === 'live' && !this.lapsed(now)
  }
}

/** Whoever keeps the entries a watch's subscriptions serve: it hears each time the Host said something of them. */
export interface WatchListener {
  heard(subscriptions: readonly Subscription[]): void
}

/** A `renew` sent and not yet answered, with when it went. */
interface SentRenewal {
  sentAt: number
  keys: string[]
}

/**
 * One watch of a Host. It connects while it carries a subscription, and
 * again after its reconnect waits when the connection ends, which ends
 * every subscription on it; a connection that brings nothing, not even a
 * heartbeat, for 75 seconds is broken (`web-application.md` § Liveness and
 * reconnection).
 */
export class WatchChannel {
  private readonly carried = new Map<string, Subscription>()
  private link: WatchLink | null = null
  private opened = false
  private waiting: ReconnectWait | null = null
  private failures = 0
  private silence: SilenceWatch | null = null
  private renewal: ReturnType<typeof setInterval> | null = null
  private renewals: SentRenewal[] = []
  /** What the views settled on this turn, sent together once they have. */
  private readonly toSubscribe = new Set<Subscription>()
  private readonly toUnsubscribe = new Set<string>()
  private flushing = false

  constructor(
    private readonly listener: WatchListener,
    private readonly open: OpenWatchLink,
    private readonly name = 'file watch',
  ) {}

  /** The watch carries `subscription` from now on: subscribed while open, and each time it opens. */
  carry(subscription: Subscription): void {
    subscription.channel = this
    this.carried.set(subscription.key, subscription)
    this.toUnsubscribe.delete(subscription.key)
    this.toSubscribe.add(subscription)
    this.flush()
  }

  /** The watch no longer carries `subscription`: unsubscribed at once, and the watch closes once it carries nothing. */
  drop(subscription: Subscription): void {
    if (this.carried.get(subscription.key) !== subscription)
      return
    this.carried.delete(subscription.key)
    subscription.channel = null
    if (!this.toSubscribe.delete(subscription))
      this.toUnsubscribe.add(subscription.key)
    this.flush()
  }

  /**
   * Subscribes again to `subscription` when the page counts its lease as
   * over, as long as the watch carries it and is open.
   */
  renewLapsed(subscription: Subscription, now = Date.now()): void {
    if (!this.opened || this.carried.get(subscription.key) !== subscription)
      return
    const status = subscription.state.status
    if ((status === 'live' || status === 'waiting') && subscription.lapsed(now) && !this.toSubscribe.has(subscription)) {
      this.toSubscribe.add(subscription)
      this.flush()
    }
  }

  /**
   * The path to the Host changed (`direct-channel.md` § Choosing the
   * path): an open watch reopens on the other path at once, as after a
   * lost connection but without its wait.
   */
  move(): void {
    if (!this.link) {
      this.online()
      return
    }
    this.end()
    this.connect()
  }

  /**
   * The watch no longer serves its Host, as a conversation's after it moved
   * to another: it lets go of everything it carries, which waits, unwatched,
   * for a view whose watch serves the Host to take it, and closes. It
   * carries again what it is given later.
   */
  retire(): void {
    const subscriptions = [...this.carried.values()]
    this.carried.clear()
    this.toSubscribe.clear()
    this.toUnsubscribe.clear()
    this.stop()
    for (const subscription of subscriptions) {
      subscription.channel = null
      subscription.leaseFrom = null
      subscription.state.status = 'waiting'
      subscription.state.reason = null
    }
    if (subscriptions.length > 0)
      this.listener.heard(subscriptions)
  }

  /** The Host came online: a watch waiting to connect again connects now. */
  online(): void {
    if (this.link || this.carried.size === 0)
      return
    this.waiting?.cancel()
    this.waiting = null
    this.connect()
  }

  /** Sends what the views settled on once they have, connecting or closing as it now carries something or nothing. */
  private flush(): void {
    if (this.flushing)
      return
    this.flushing = true
    queueMicrotask(() => {
      this.flushing = false
      if (this.carried.size === 0) {
        this.toSubscribe.clear()
        this.toUnsubscribe.clear()
        this.stop()
        return
      }
      if (!this.link && !this.waiting) {
        this.connect()
        return
      }
      if (!this.opened)
        return
      this.sendUnsubscribe([...this.toUnsubscribe])
      this.toUnsubscribe.clear()
      this.subscribe([...this.toSubscribe])
      this.toSubscribe.clear()
    })
  }

  private connect(): void {
    if (this.carried.size === 0)
      return
    let link: WatchLink | null = null
    const current = () => link !== null && this.link === link
    link = this.open({
      opened: () => {
        if (!current())
          return
        this.opened = true
        this.silence = watchSilence(this.name, () => this.lose(link!))
        this.renewal = setInterval(() => this.renew(), RENEW_MS)
        this.toSubscribe.clear()
        this.toUnsubscribe.clear()
        this.subscribe([...this.carried.values()])
      },
      message: (message) => {
        if (!current())
          return
        this.silence?.heard()
        this.receive(message)
      },
      closed: () => this.lose(link!),
    })
    this.link = link
  }

  private subscribe(subscriptions: Subscription[]): void {
    const now = Date.now()
    for (const subscription of subscriptions) {
      subscription.subscribedAt = now
      subscription.state.status = 'waiting'
      subscription.state.reason = null
    }
    for (let at = 0; at < subscriptions.length; at += MESSAGE_KEYS) {
      this.link?.send({
        type: 'subscribe',
        subscriptions: subscriptions.slice(at, at + MESSAGE_KEYS).map(({ key, subject }) => ({ key, ...subject })),
      })
    }
    if (subscriptions.length > 0)
      this.listener.heard(subscriptions)
  }

  private sendUnsubscribe(keys: string[]): void {
    for (let at = 0; at < keys.length; at += MESSAGE_KEYS)
      this.link?.send({ type: 'unsubscribe', keys: keys.slice(at, at + MESSAGE_KEYS) })
  }

  /**
   * Every 3 minutes: the subscriptions the Host still holds are renewed in
   * one message, and those whose lease the page counts as over are
   * subscribed again.
   */
  private renew(): void {
    const now = Date.now()
    const held = [...this.carried.values()].filter((subscription) =>
      subscription.state.status === 'live' || subscription.state.status === 'waiting')
    const lapsed = held.filter((subscription) => subscription.lapsed(now))
    const keys = held.filter((subscription) => !subscription.lapsed(now)).map((subscription) => subscription.key)
    for (let at = 0; at < keys.length; at += MESSAGE_KEYS) {
      const part = keys.slice(at, at + MESSAGE_KEYS)
      this.renewals.push({ sentAt: now, keys: part })
      this.link?.send({ type: 'renew', keys: part })
    }
    this.subscribe(lapsed)
  }

  private receive(message: WatchMessage): void {
    switch (message.type) {
      case 'heartbeat':
        return
      case 'live': {
        const subscription = this.carried.get(message.key)
        if (!subscription)
          return
        this.failures = 0
        // Another run of the runner: its numbers start over.
        if (subscription.epoch !== message.epoch)
          subscription.changedAt = -1
        subscription.epoch = message.epoch
        subscription.since = message.since
        subscription.lives += 1
        // The lease runs from the subscribe this answers.
        subscription.leaseFrom = Math.max(subscription.leaseFrom ?? 0, subscription.subscribedAt)
        subscription.state.status = 'live'
        subscription.state.reason = null
        this.listener.heard([subscription])
        return
      }
      case 'changed': {
        const heard: Subscription[] = []
        for (const [key, at] of message.changes) {
          const subscription = this.carried.get(key)
          if (!subscription)
            continue
          subscription.changedAt = Math.max(subscription.changedAt, at)
          heard.push(subscription)
        }
        if (heard.length > 0)
          this.listener.heard(heard)
        return
      }
      case 'unavailable':
      case 'refused': {
        const subscription = this.carried.get(message.key)
        if (!subscription)
          return
        subscription.state.status = message.type
        subscription.state.reason = message.type === 'refused' ? REFUSED_REASON : message.reason
        subscription.leaseFrom = null
        this.listener.heard([subscription])
        return
      }
      case 'renewed': {
        // Answers come in the order the renews went.
        const renewal = this.renewals.shift()
        if (!renewal)
          return
        for (const key of message.keys) {
          const subscription = this.carried.get(key)
          if (subscription)
            subscription.leaseFrom = Math.max(subscription.leaseFrom ?? 0, renewal.sentAt)
        }
        return
      }
      case 'offline':
        // The socket closes after it.
        this.unwatch()
    }
  }

  /** Nothing on the watch is watched any more: each subscription waits for the next connection's `live`. */
  private unwatch(): void {
    const subscriptions = [...this.carried.values()]
    for (const subscription of subscriptions) {
      subscription.leaseFrom = null
      if (subscription.state.status === 'live')
        subscription.state.status = 'waiting'
    }
    if (subscriptions.length > 0)
      this.listener.heard(subscriptions)
  }

  /** Ends the connection there is, with its silence watch and its renewals; every subscription on it ended with it. */
  private end(): void {
    const link = this.link
    this.link = null
    this.opened = false
    this.silence?.stop()
    this.silence = null
    if (this.renewal !== null)
      clearInterval(this.renewal)
    this.renewal = null
    this.renewals = []
    // A broken socket's own close may come much later, or never.
    link?.close()
    this.unwatch()
  }

  /** The connection closed, or is taken as broken: it connects again after its wait while it carries something. */
  private lose(link: WatchLink): void {
    if (this.link !== link)
      return
    this.end()
    if (this.carried.size === 0)
      return
    this.failures += 1
    this.waiting = waitToReconnect(this.failures, () => {
      this.waiting = null
      this.connect()
    })
  }

  /** It carries nothing: the connection closes, and a wait to connect again ends. */
  private stop(): void {
    this.waiting?.cancel()
    this.waiting = null
    if (this.link)
      this.end()
  }
}
