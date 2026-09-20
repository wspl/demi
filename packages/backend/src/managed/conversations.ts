import type { AgentServer } from '@demicodes/agent'
import type { ConversationTargets } from '../conversation/target'
import type { ControlService } from '../storage/control'

export interface CloudConversationReservation {
  release(): void
}

/** How a conversation uses a device (`sessions-and-targets.md` § How a conversation uses a device). */
export type DeviceUse = 'target' | 'provider' | 'attached'

/**
 * The conversations a Cloud's lifecycle reaches: those that use it, in any
 * role, keep it awake, and those that cannot work without it — it is their
 * target, or where their provider's process runs — are held while its
 * generation retires. The lifecycle knows roles; it does not know providers.
 */
export class CloudConversations {
  constructor(private readonly options: {
    control: ControlService
    targets(): ConversationTargets
    agents(): AgentServer
    /**
     * Whether the conversation's selected provider runs a process on the
     * user's Cloud, which the placement decides (`claude-cli.md` § Where it runs).
     */
    providerOnCloud(conversationId: string): Promise<boolean>
  }) {}

  async active(userId: string): Promise<boolean> {
    return [...(await this.uses(userId)).keys()].some(id =>
      this.options.agents().treeActive(id) ||
      this.options.targets().files(id).demandActive
    )
  }

  observe(userId: string, changed: (demand?: boolean) => void): () => void {
    let closed = false
    const observe = (id: string, active: boolean) => {
      void this.uses(userId).then(uses => {
        if (!closed && uses.has(id)) changed(active)
      }).catch(error => console.error('Cloud activity observation failed', error))
    }
    const unsubscribeTree = this.options.agents().observeActivity(observe)
    const unsubscribeFiles = this.options.targets().observeActivity(observe)
    return () => {
      closed = true
      unsubscribeTree()
      unsubscribeFiles()
    }
  }

  async reserve(userId: string, reason: 'idle' | 'reset'): Promise<CloudConversationReservation | null> {
    using reservations = new DisposableStack()
    const targets = this.options.targets()
    const agents = this.options.agents()
    for (const [id, use] of await this.uses(userId)) {
      // A conversation with the Cloud only attached runs on its own target and is left alone.
      if (use === 'attached')
        continue
      const tree = reason === 'reset'
        ? await agents.interruptTree(id)
        : agents.reserveTreeMutation(id)
      if (!tree) return null
      reservations.defer(tree)
      // A reset ends the transfers of a conversation whose files are on the
      // device, with the device's other work; an idle stop has none, since a
      // transfer is activity. A conversation that only infers through the
      // device keeps its transfers: its files are elsewhere.
      if (reason === 'reset' && use === 'target')
        reservations.defer(await targets.closeTransfers(id))
      const file = reason === 'reset'
        ? await targets.files(id).reserve()
        : targets.files(id).tryReserve()
      if (!file) return null
      reservations.defer(file)
    }
    const held = reservations.move()
    return {
      release: () => held.dispose(),
    }
  }

  /** Each of the user's conversations that uses the Cloud, with its strongest role. */
  private async uses(userId: string): Promise<Map<string, DeviceUse>> {
    const ids = await this.options.control.listUserConversationIds(userId)
    const device = await this.options.control.getManagedDevice(userId)
    const uses = new Map<string, DeviceUse>()
    for (const id of ids) {
      const target = await this.options.targets().resolve(id)
      if (target.kind === 'cloud' || target.deviceId === device?.id) {
        uses.set(id, 'target')
        continue
      }
      if (await this.options.providerOnCloud(id)) {
        uses.set(id, 'provider')
        continue
      }
      if (!device)
        continue
      const attached = await this.options.control.listAttachedHosts(id)
      if (attached.some(host => host.deviceId === device.id))
        uses.set(id, 'attached')
    }
    return uses
  }
}
