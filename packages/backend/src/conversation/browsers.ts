import { BROWSER_IDLE_MS, BROWSER_RESOURCE_KIND } from '@demicodes/browser-protocol'
import type { NativePackage, NativeResourceGrant, ArtifactResolver } from '@demicodes/command-protocol'
import type { RemoteHost } from '@demicodes/host-remote'
import type { ConversationTargets } from './target'
import type { LifecycleCoordinator } from '../lifecycle/coordinator'
import { noop } from '@demicodes/utils'

interface BrowserOwner {
  host: RemoteHost
  grant: NativeResourceGrant
  dispose(): void
}

/** A root conversation retains one native browser grant across its entire agent tree. */
export class ConversationBrowsers {
  private readonly owners = new Map<string, BrowserOwner>()

  constructor(private readonly options: {
    targets: ConversationTargets
    descriptor: NativePackage
    resolveArtifact: ArtifactResolver
    lifecycle: LifecycleCoordinator
    treeActive(id: string): boolean
    observeTree(id: string, changed: () => void): () => void
    reserveTree(id: string): (() => void) | null
    idleMs?: number
  }) {}

  async run<T>(
    conversationId: string,
    expectedHost: RemoteHost,
    signal: AbortSignal,
    operation: (resources: readonly NativeResourceGrant[]) => Promise<T>,
  ): Promise<T> {
    return this.options.targets.withHost(conversationId, async host => {
      if (host !== expectedHost)
        throw new Error('Conversation Host changed before this shell job started')
      let owner = this.owners.get(conversationId)
      if (owner && (owner.host !== host || !host.resourceLive(owner.grant))) {
        if (owner.host.resourceLive(owner.grant))
          throw new Error('Previous browser resource has not been retired')
        this.forget(conversationId, owner)
        owner = undefined
      }
      if (!owner) {
        const grant = await host.acquireResource(conversationId, BROWSER_RESOURCE_KIND, this.options.descriptor, this.options.resolveArtifact, signal)
        owner = this.owners.get(conversationId)
        if (!owner || owner.grant.scope.id !== grant.scope.id)
          owner = this.remember(conversationId, host, grant)
      }
      try {
        return await operation([owner.grant])
      } finally {
        // The Host admission also covers the final status/release exchange, after
        // shell output and edit publication have completed.
        const status = await host.resourceStatus(owner.grant)
        if (status.state === 'released') {
          await host.releaseResource(owner.grant)
          if (this.owners.get(conversationId)?.grant.scope.id === owner.grant.scope.id)
            this.forget(conversationId, owner)
        }
      }
    }, { signal })
  }

  async retire(id: string, fileReservation: () => void, deviceReservation?: () => void): Promise<void> {
    const owner = this.owners.get(id)
    if (!owner) return
    if (owner.host.resourceLive(owner.grant)) {
      try {
        await this.options.lifecycle.cleanup({ conversationId: id, host: owner.host, grant: owner.grant, fileReservation, deviceReservation }, cleanup =>
          this.options.targets.withHost(id, host => host.releaseResource(owner.grant), { cleanup }))
      } catch (error) {
        // Connection/resource loss is authoritative and owns remote teardown. An
        // unconfirmed release on a live generation must still block the transition.
        if (owner.host.resourceLive(owner.grant)) throw error
      }
    }
    this.forget(id, owner)
  }

  async close(): Promise<void> {
    for (const id of [...this.owners.keys()]) {
      const release = await this.options.targets.files(id).reserve('forced')
      try {
        await this.retire(id, release)
      } finally {
        release()
      }
    }
  }

  private remember(id: string, host: RemoteHost, grant: NativeResourceGrant): BrowserOwner {
    const owner: BrowserOwner = { host, grant, dispose: noop }
    this.owners.set(id, owner)
    const stopResources = host.observeResources(() => {
      if (!host.resourceLive(grant)) this.forget(id, owner)
    })
    const stopIdle = this.options.lifecycle.idle({
      idleMs: this.options.idleMs ?? BROWSER_IDLE_MS,
      demand: () => this.options.treeActive(id) || this.options.targets.files(id).demandActive,
      eligible: () => host.resourceLive(grant) && !this.options.treeActive(id) && !this.options.targets.files(id).demandActive,
      observe: changed => {
        const tree = this.options.observeTree(id, changed)
        const files = this.options.targets.files(id).subscribe(changed)
        return () => {
          tree()
          files()
        }
      },
      reserve: () => {
        const tree = this.options.reserveTree(id)
        if (!tree) return null
        const files = this.options.targets.files(id).tryReserve('idle')
        if (!files) {
          tree()
          return null
        }
        return {
          run: () => this.retire(id, files),
          release: () => {
            files()
            tree()
          },
        }
      },
    })
    owner.dispose = () => {
      stopIdle()
      stopResources()
    }
    return owner
  }

  private forget(id: string, owner: BrowserOwner): void {
    if (this.owners.get(id) !== owner) return
    this.owners.delete(id)
    owner.dispose()
  }
}
