import type { AgentServer } from '@demicodes/agent'
import type { ConversationTargets } from '../conversation/target'
import type { ConversationBrowsers } from '../conversation/browsers'
import type { ControlService } from '../storage/control'

export interface CloudConversationReservation {
  retire(deviceReservation: () => void): Promise<void>
  release(): void
}

/** Reserves the conversations that depend on Cloud before its generation retires. */
export class CloudConversations {
  constructor(private readonly options: {
    control: ControlService
    targets(): ConversationTargets
    agents(): AgentServer
    browsers(): ConversationBrowsers | null
  }) {}

  async active(userId: string): Promise<boolean> {
    return (await this.selected(userId)).some(id => this.options.agents().treeActive(id))
  }

  observe(userId: string, changed: (demand?: boolean) => void): () => void {
    let closed = false
    const unsubscribe = this.options.agents().observeActivity((id, active) => {
      void this.selected(userId).then(ids => {
        if (!closed && ids.includes(id)) changed(active)
      }).catch(error => console.error('Cloud activity observation failed', error))
    })
    return () => {
      closed = true
      unsubscribe()
    }
  }

  async reserve(userId: string, reason: 'idle' | 'reset'): Promise<CloudConversationReservation | null> {
    using reservations = new DisposableStack()
    const files = new Map<string, () => void>()
    const targets = this.options.targets()
    const agents = this.options.agents()
    for (const id of await this.selected(userId)) {
      const tree = reason === 'reset'
        ? await agents.interruptTree(id)
        : agents.reserveTreeMutation(id, 'idle')
      if (!tree) return null
      reservations.defer(tree)
      const file = reason === 'reset'
        ? await targets.files(id).reserve('forced')
        : targets.files(id).tryReserve('idle')
      if (!file) return null
      reservations.defer(file)
      files.set(id, file)
    }
    const held = reservations.move()
    return {
      retire: async deviceReservation => {
        const device = await this.options.control.getManagedDevice(userId)
        const browsers = this.options.browsers()
        if (!device || !browsers) return
        for (const [id, file] of files) {
          const target = await targets.resolve(id)
          if (target.kind === 'cloud' || target.deviceId === device.id)
            await browsers.retire(id, file, deviceReservation)
        }
      },
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
