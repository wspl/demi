import { computed, shallowRef, triggerRef, type ComputedRef, type ShallowRef } from 'vue'

/** A revision for each part of a slot a write changed, by part. */
export type Revisions = Readonly<Record<string, number>>

/**
 * What a write's answer says of the slot (`web-application.md` § Responding
 * to the user).
 */
export interface SlotAnswer<V> {
  /**
   * The value the answer leaves, made of the value read, when the answer
   * tells the parts its change touches whole, as a product state's part is
   * told. It is taken only when no read of those parts came since the write
   * was sent, since that read may be newer.
   */
  value?: (read: V) => V
  /**
   * The revision of each part the write produced. The change leaves once the
   * page holds a read of each at that revision or a later one, never
   * earlier: an answer that adds one item to a list is not the list.
   */
  revisions?: Revisions
}

/** Where a change stands: shown and not sent yet, sent, or answered and waiting for a read that holds it. */
export type SlotChangeStatus = 'unsent' | 'sent' | 'landed'

/**
 * One change of the page's that its write has not landed yet: shown over the
 * slot's value from the moment the user made it, in the order made.
 */
export class SlotChange<V, M = undefined> {
  status: SlotChangeStatus = 'unsent'
  /** Where the slot's clock stood when the write went out. */
  sentAt: number | null = null
  /** Where the slot's clock stood when the answer came. */
  landedAt: number | null = null
  /** The revisions the answer named, which a read must reach for the change to leave. */
  until: Revisions | null = null

  constructor(
    private readonly slot: Slot<V, M>,
    private shownAs: (value: V) => V,
    /** The parts of the value the change touches. */
    readonly parts: readonly string[],
    /** What the owner of the slot keeps with the change. */
    readonly meta: M,
  ) {}

  /** How the change shows over the value below it. */
  get apply(): (value: V) => V {
    return this.shownAs
  }

  /** Shows the change as `apply` from now on, as more typing does to a change not sent yet. */
  revise(apply: (value: V) => V): void {
    this.shownAs = apply
    this.slot.touched()
  }

  send(): void {
    this.slot.send([this])
  }

  /** The write was answered (`Slot.land`); answers whether the change left. */
  land(answer?: SlotAnswer<V>): boolean {
    return this.slot.land([this], answer)
  }

  /** The write failed or the backend refused it: the change leaves at once (`Slot.drop`). */
  drop(answer?: SlotAnswer<V>): void {
    this.slot.drop([this], answer)
  }
}

/**
 * A piece of state the page reads from the backend (`web-application.md`
 * § Responding to the user): the value it last read, the revision of each of
 * its parts, and the changes the page made and has not seen land. The page
 * shows `shown`, the value read with those changes applied in the order they
 * were made. One mechanism serves every slot:
 *
 * - A change leaves when the page holds a read at or after the revisions its
 *   answer named; an answer without revisions that tells its parts whole is
 *   taken as their value when no read came since the write, else the change
 *   waits for a read of its parts asked for after the answer. An answer
 *   releases only its own change, never another.
 * - A change whose effect the value read already holds, such as the removal
 *   of an item the read no longer has, leaves once answered.
 * - A change whose write failed or was refused leaves at once; the page then
 *   shows what the backend has, and the slot's owner keeps what the user
 *   wrote in it.
 *
 * The writes of one thing go one at a time, in the order the user made them,
 * which their owner orders, such as with a `SerialQueue`.
 */
export class Slot<V, M = undefined> {
  /** The value as the backend last told it; null before the first read. */
  readonly read: ShallowRef<V | null> = shallowRef(null)
  /** The changes not left yet, in the order they were made. */
  private readonly changes = shallowRef<SlotChange<V, M>[]>([])
  /** The revision of each part as read. */
  private readonly held = shallowRef<Record<string, number>>({})
  private clock = 0
  /** The clock at each part's last read, and at the last read of the whole. */
  private readonly partAt = new Map<string, number>()
  private wholeAt = 0
  /** The newest revision of each part a read was started for. */
  private readonly asked = new Map<string, number>()
  /** The value the page shows: the value read, or `empty` before the first read, with its changes on top. */
  readonly shown: ComputedRef<V | null>

  /** `empty` is the value to show before the first read, for a slot whose first read is a list that is nothing yet. */
  constructor(private readonly empty: V | null = null) {
    this.shown = computed(() => {
      const base = this.read.value ?? this.empty
      return base === null ? null : this.changes.value.reduce<V>((value, change) => change.apply(value), base)
    })
  }

  /** The revision of `part` as last read; -1 before the first. */
  revision(part = ''): number {
    return this.held.value[part] ?? -1
  }

  /** The newest revision of `part` held or asked for: a read of an older one is not worth starting. */
  requested(part = ''): number {
    return Math.max(this.revision(part), this.asked.get(part) ?? -1)
  }

  /**
   * Whether a read of `part` at `revision` is worth starting: it is newer
   * than the value held and than any read started before, which it then
   * counts as.
   */
  ask(revision: number, part = ''): boolean {
    if (revision <= this.requested(part)) {
      return false
    }
    this.asked.set(part, revision)
    return true
  }

  /** The slot's clock, which numbers its reads: a write sent or a read asked for at this point is measured against it. */
  now(): number {
    return this.clock
  }

  /** The changes not left yet that touch `part`, in order. */
  pending(part?: string): readonly SlotChange<V, M>[] {
    return this.changes.value.filter((change) => part === undefined || change.parts.includes(part))
  }

  /** The value `change` shows over: the value read with the changes made before it. */
  below(change: SlotChange<V, M>): V | null {
    const base = this.read.value ?? this.empty
    const before = this.changes.value.slice(0, Math.max(0, this.changes.value.indexOf(change)))
    return base === null ? null : before.reduce<V>((value, each) => each.apply(value), base)
  }

  /**
   * Takes a read of the whole value, as a snapshot or a request answers. A
   * read of no newer revision than the one held changes nothing. `since` is
   * where the clock stood when the request for it went out; a read from the
   * channel needs none. Answers whether it was taken.
   */
  replace(value: V, options: { revisions?: Revisions; since?: number } = {}): boolean {
    const { revisions } = options
    if (revisions !== undefined && this.read.value !== null
      && Object.entries(revisions).every(([part, revision]) => revision <= this.revision(part))) {
      return false
    }
    const at = ++this.clock
    this.wholeAt = at
    this.read.value = value
    if (revisions !== undefined) {
      this.raise(revisions)
    }
    this.release(() => true, options.since ?? at)
    return true
  }

  /**
   * Takes a read of one part, as a message of the channel brings it:
   * `update` makes the value of the one it replaces. Nothing is read before
   * the whole, and a read of no newer revision than the one held changes
   * nothing. Answers whether it was taken.
   */
  receive(part: string, update: (read: V) => V, options: { revision?: number; since?: number } = {}): boolean {
    const read = this.read.value
    const { revision } = options
    if (read === null || (revision !== undefined && revision <= this.revision(part))) {
      return false
    }
    const at = ++this.clock
    this.partAt.set(part, at)
    this.read.value = update(read)
    if (revision !== undefined) {
      this.raise({ [part]: revision })
    }
    this.release((change) => change.parts.includes(part), options.since ?? at)
    return true
  }

  /**
   * Applies what a write's answer says of `parts`, for a write no change
   * stands for, unless one of them was read since the write was sent `at`:
   * that read may be newer, and if it is older, the value the write caused
   * is still to come. Releases no change. Answers whether it applied.
   */
  answered(parts: readonly string[], at: number, answer: SlotAnswer<V> & { value: (read: V) => V }): boolean {
    const read = this.read.value
    if (read === null || this.readSince(parts, at)) {
      return false
    }
    this.read.value = answer.value(read)
    if (answer.revisions !== undefined) {
      this.raise(answer.revisions)
    }
    // A raised revision is a read of what the answer holds: only the
    // changes whose answers named it leave.
    this.release(() => false, -1)
    return true
  }

  /**
   * Shows `apply`, a change the user made of `parts`, over the value at once,
   * until its write lands. The caller sends the write, after the writes
   * before it, and says how it ended through the change.
   */
  change(apply: (value: V) => V, options: { parts?: readonly string[]; meta?: M } = {}): SlotChange<V, M> {
    const change = new SlotChange<V, M>(this, apply, options.parts ?? [''], options.meta as M)
    this.changes.value = [...this.changes.value, change]
    return change
  }

  /** The writes of `changes` go out now, together. */
  send(changes: readonly SlotChange<V, M>[]): void {
    for (const change of changes) {
      change.status = 'sent'
      change.sentAt = this.clock
    }
  }

  /**
   * The write of `changes`, which went out together, was answered;
   * without an answer, the changes applied in order are the value of their
   * parts. Answers whether the changes left.
   */
  land(changes: readonly SlotChange<V, M>[], answer?: SlotAnswer<V>): boolean {
    // A change released since, as by a sign-out, has nothing left to say,
    // and one that landed waits only for a read.
    const live = changes.filter((change) => this.changes.value.includes(change) && change.status !== 'landed')
    if (live.length === 0) {
      return true
    }
    const said = answer ?? { value: (read: V) => live.reduce((value, change) => change.apply(value), read) }
    const parts = [...new Set(live.flatMap((change) => change.parts))]
    const at = Math.min(...live.map((change) => change.sentAt ?? this.clock))
    const landedAt = ++this.clock
    for (const change of live) {
      change.status = 'landed'
      change.landedAt = landedAt
      change.until = said.revisions === undefined
        ? null
        : Object.fromEntries(Object.entries(said.revisions).filter(([part]) => change.parts.includes(part)))
    }
    const read = this.read.value
    if (said.value !== undefined && read !== null && !this.readSince(parts, at)) {
      this.read.value = said.value(read)
      if (said.revisions !== undefined) {
        this.raise(said.revisions)
      }
      // The value is the write's: its own changes leave, and only they.
      this.leave(live.filter((change) => change.until === null || this.reached(change.until)))
    }
    this.release(() => false, -1)
    return !live.some((change) => this.changes.value.includes(change))
  }

  /**
   * The write of `changes` failed or was refused: they leave, and `answer`,
   * what the backend has, applies as in `answered`. A change that landed
   * waits only for a read, and stays.
   */
  drop(changes: readonly SlotChange<V, M>[], answer?: SlotAnswer<V>): void {
    const live = changes.filter((change) => this.changes.value.includes(change) && change.status !== 'landed')
    if (live.length === 0) {
      return
    }
    if (answer?.value !== undefined) {
      const at = Math.min(...live.map((change) => change.sentAt ?? this.clock))
      this.answered([...new Set(live.flatMap((change) => change.parts))], at, { ...answer, value: answer.value })
    }
    this.leave(live)
  }

  /** `left` leave without a write, as a change another one took the place of. */
  leave(left: readonly SlotChange<V, M>[]): void {
    if (left.some((change) => this.changes.value.includes(change))) {
      this.changes.value = this.changes.value.filter((change) => !left.includes(change))
    }
  }

  /** A change was revised: the value shown is made again. */
  touched(): void {
    triggerRef(this.changes)
  }

  /** Forgets everything, as a page that signed out does. */
  clear(): void {
    this.read.value = null
    this.changes.value = []
    this.held.value = {}
    this.clock = 0
    this.partAt.clear()
    this.wholeAt = 0
    this.asked.clear()
  }

  /** Whether one of `parts` was read after the clock stood at `at`. */
  private readSince(parts: readonly string[], at: number): boolean {
    return Math.max(this.wholeAt, ...parts.map((part) => this.partAt.get(part) ?? 0)) > at
  }

  private raise(revisions: Revisions): void {
    const held = { ...this.held.value }
    for (const [part, revision] of Object.entries(revisions)) {
      held[part] = Math.max(held[part] ?? -1, revision)
    }
    this.held.value = held
  }

  /** Whether the page holds a read of every part `until` names at that revision or a later one. */
  private reached(until: Revisions): boolean {
    return Object.entries(until).every(([part, revision]) => this.revision(part) >= revision)
  }

  /**
   * The landed changes a read now holds leave: those whose revisions it
   * reached; those without revisions of which `of` read a part, when the
   * request for the read went out at `since`, after they landed; and those
   * whose effect the value read already has.
   */
  private release(of: (change: SlotChange<V, M>) => boolean, since: number): void {
    const read = this.read.value
    const released = (change: SlotChange<V, M>) => {
      if (change.status !== 'landed') {
        return false
      }
      if (change.until !== null) {
        return this.reached(change.until)
      }
      if (of(change) && since >= change.landedAt!) {
        return true
      }
      return read !== null && change.apply(read) === read
    }
    this.leave(this.changes.value.filter(released))
  }
}
