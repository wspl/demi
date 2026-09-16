import { errorMessage } from '@demicodes/utils'

export interface Retirement {
  run(): Promise<void>
  release(): void
}

interface IdlePolicy {
  idleMs: number
  eligible(): boolean | Promise<boolean>
  demand?(): boolean
  pollMs?: number
  observe(changed: (demand?: boolean) => void): () => void
  reserve(): Retirement | null | Promise<Retirement | null>
}

interface Scheduled {
  deadline(): number | null
  run(): Promise<void>
  dispose(): void
  transition?: Promise<void>
}

export const CONVERSATION_IDLE_MS = 60 * 60_000

/** One backend clock schedules conversation retirement and Cloud maintenance. */
export class LifecycleCoordinator {
  private readonly scheduled = new Set<Scheduled>()
  private readonly transitions = new Set<Promise<void>>()
  private timer: ReturnType<typeof setTimeout> | undefined
  private closed = false

  constructor(private readonly now = () => performance.now(), private readonly report: (message: string) => void = console.error) {}

  periodic(intervalMs: number, operation: () => Promise<void>): () => void {
    let next = this.now() + intervalMs
    return this.add({
      deadline: () => next,
      run: async () => {
        try {
          await operation()
        } finally {
          next = this.now() + intervalMs
        }
      },
      dispose: () => {},
    })
  }

  idle(policy: IdlePolicy): () => void {
    let idleSince: number | null = null
    let nextCheck = this.now()
    let deferred = false
    let revision = 0
    const refresh = (demand = false) => {
      revision++
      if (demand || policy.demand?.()) idleSince = null
      deferred = false
      nextCheck = this.now()
      this.schedule()
    }
    const unsubscribe = policy.observe(refresh)
    const entry: Scheduled = {
      deadline: () => Math.min(nextCheck, idleSince === null || deferred ? Infinity : idleSince + policy.idleMs),
      run: async () => {
        let retirement: Retirement | null = null
        const inspectedRevision = revision
        nextCheck = Infinity
        try {
          const eligible = await policy.eligible()
          if (revision !== inspectedRevision) {
            nextCheck = this.now()
            return
          }
          nextCheck = policy.pollMs === undefined ? Infinity : this.now() + policy.pollMs
          if (!eligible) {
            idleSince = null
            return
          }
          idleSince ??= this.now()
          if (this.now() < idleSince + policy.idleMs) return
          retirement = await policy.reserve()
          if (!retirement) {
            // Maintenance preserves the elapsed interval; its release observer
            // retries admission, without a timer spinning on an expired deadline.
            deferred = true
            return
          }
          if (!await policy.eligible()) {
            idleSince = null
            return
          }
          // Demand can finish while an asynchronous reservation is pending.
          // Its reset still invalidates the elapsed idle interval.
          if (idleSince === null || this.now() < idleSince + policy.idleMs) return
          await retirement.run()
          this.remove(entry)
        } catch (error) {
          retirement?.release()
          retirement = null
          deferred = true
          nextCheck = this.now() + (policy.pollMs ?? policy.idleMs)
          throw error
        } finally {
          retirement?.release()
        }
      },
      dispose: unsubscribe,
    }
    const remove = this.add(entry)
    return remove
  }

  async close(): Promise<void> {
    this.closed = true
    if (this.timer !== undefined) clearTimeout(this.timer)
    this.timer = undefined
    const entries = [...this.scheduled]
    for (const entry of entries) this.remove(entry)
    await Promise.all(this.transitions)
  }

  private add(entry: Scheduled): () => void {
    if (this.closed) throw new Error('Lifecycle coordinator is closed')
    this.scheduled.add(entry)
    this.schedule()
    return () => this.remove(entry)
  }

  private remove(entry: Scheduled): void {
    if (!this.scheduled.delete(entry)) return
    entry.dispose()
    this.schedule()
  }

  private schedule(): void {
    if (this.timer !== undefined) clearTimeout(this.timer)
    this.timer = undefined
    if (this.closed) return
    let next = Infinity
    for (const entry of this.scheduled) {
      if (!entry.transition) next = Math.min(next, entry.deadline() ?? Infinity)
    }
    if (!Number.isFinite(next)) return
    this.timer = setTimeout(() => {
      this.timer = undefined
      const now = this.now()
      for (const entry of this.scheduled) {
        const deadline = entry.deadline()
        if (entry.transition || deadline === null || deadline > now) continue
        const transition = Promise.resolve().then(() => entry.run())
          .catch(error => this.report(errorMessage(error)))
          .finally(() => {
            entry.transition = undefined
            this.transitions.delete(transition)
            this.schedule()
          })
        entry.transition = transition
        this.transitions.add(transition)
      }
      this.schedule()
    }, Math.max(0, next - this.now()))
  }
}
