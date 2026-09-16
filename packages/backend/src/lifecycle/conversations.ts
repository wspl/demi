import { reachableHosts } from '../conversation/hosts'
import type { ConversationTargets } from '../conversation/target'
import type { ControlService } from '../storage/control'
import type { RunnerRegistry } from '../runner/registry'
import type { LifecycleCoordinator } from './coordinator'

/** Schedule conversation state release after idle and join binding-transition cleanup. */
export class ConversationLifecycle {
  private readonly idle = new Map<string, () => void>()

  constructor(private readonly options: {
    control: ControlService
    registry: RunnerRegistry
    targets(): ConversationTargets
    lifecycle: LifecycleCoordinator
    idleMs: number
    idlePollMs?: number
    treeActive(id: string): boolean
    observeTree(id: string, changed: (active?: boolean) => void): () => void
    reserveTree(id: string): (() => void) | null
  }) {}

  async release(id: string, deviceId?: string): Promise<void> {
    const devices = (await reachableHosts(this.options, id))
      .filter(host => !deviceId || host.deviceId === deviceId)
    const results = await Promise.allSettled(devices.map(device =>
      this.options.targets().withHost(id, host => host.releaseConversation(id), {
        lifecycle: true,
        deviceId: device.deviceId,
      })
    ))
    const errors = results.flatMap(result => result.status === 'rejected' ? [result.reason] : [])
    if (errors.length) throw new AggregateError(errors, 'Conversation release failed')
    if (!deviceId) {
      this.idle.get(id)?.()
      this.idle.delete(id)
    }
  }

  /** A paired conversation retains state only until its next full idle window. */
  track(id: string): void {
    const lifecycle = this.options.lifecycle
    if (this.idle.has(id)) return
    const files = this.options.targets().files(id)
    const active = () => files.demandActive || this.options.treeActive(id)
    const stop = lifecycle.idle({
      idleMs: this.options.idleMs,
      pollMs: this.options.idlePollMs ?? 30_000,
      demand: active,
      eligible: () => !active(),
      observe: changed => {
        const unobserveFiles = files.subscribe(() => changed(files.demandActive))
        const unobserveTree = this.options.observeTree(id, changed)
        return () => {
          unobserveFiles()
          unobserveTree()
        }
      },
      reserve: () => {
        const tree = this.options.reserveTree(id)
        if (!tree) return null
        const file = files.tryReserve()
        if (!file) {
          tree()
          return null
        }
        return {
          run: () => this.release(id),
          release: () => {
            file()
            tree()
          },
        }
      },
    })
    this.idle.set(id, stop)
  }

  reset(id: string): void {
    this.idle.get(id)?.()
    this.idle.delete(id)
    this.track(id)
  }

  close(): void {
    for (const stop of this.idle.values()) stop()
    this.idle.clear()
  }
}
