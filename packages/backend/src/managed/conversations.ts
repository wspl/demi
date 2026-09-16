import type { AgentServer } from '@demicodes/agent'
import type { ConversationTargets } from '../conversation/target'
import type { ControlService } from '../storage/control'

export interface CloudConversationReservation {
  release(): void
}

/** Reserves the conversations that depend on Cloud before its generation retires. */
export class CloudConversations {
  constructor(private readonly options: {
    control: ControlService
    targets(): ConversationTargets
    agents(): AgentServer
  }) {}

  async active(userId: string): Promise<boolean> {
    return (await this.selected(userId)).some(id =>
      this.options.agents().treeActive(id) ||
      this.options.targets().files(id).demandActive
    )
  }

  observe(userId: string, changed: (demand?: boolean) => void): () => void {
    let closed = false
    const observe = (id: string, active: boolean) => {
      void this.selected(userId).then(ids => {
        if (!closed && ids.includes(id)) changed(active)
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
    for (const id of await this.selected(userId)) {
      const tree = reason === 'reset'
        ? await agents.interruptTree(id)
        : agents.reserveTreeMutation(id)
      if (!tree) return null
      reservations.defer(tree)
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

  private async selected(userId: string): Promise<string[]> {
    const ids = await this.options.control.listUserConversationIds(userId)
    const device = await this.options.control.getManagedDevice(userId)
    const selected: string[] = []
    for (const id of ids) {
      const target = await this.options.targets().resolve(id)
      const attached = device ? await this.options.control.listAttachedHosts(id) : []
      if (target.kind === 'cloud' || target.deviceId === device?.id ||
        attached.some(host => host.deviceId === device?.id))
        selected.push(id)
    }
    return selected
  }
}
