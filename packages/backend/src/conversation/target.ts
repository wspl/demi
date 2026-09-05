import { rm } from 'node:fs/promises'
import { join } from 'node:path'
import type { SessionPhase } from '@demicodes/core'
import type { VirtualHost } from '@demicodes/host-virtual'
import { RemoteHost } from '@demicodes/host-remote'
import type { Host } from '@demicodes/shell'
import { noop } from '@demicodes/utils'
import { ManagedHostError, type ManagedHosts, ownerOf } from '../managed/lifecycle'
import type { RunnerRegistry } from '../runner/registry'
import type { ControlService, DeviceKind, ExecutionTarget, WorkspaceRecord } from '../storage/control'
import type { ConversationStores } from '../storage/conversation-store'
import { resolveExecutionTarget, targetDeviceId } from './execution-target'
import { HOSTLESS_HOME } from './scoped-transport'

export interface ConversationTargetsDeps {
  control: ControlService
  registry: RunnerRegistry
  /** Null when this backend provisions no machines: the upgrade then fails as an ordinary tool error. */
  managedHosts: ManagedHosts | null
  virtualHostFor: (conversationId: string) => Promise<VirtualHost>
  stores: ConversationStores
  /** Where a conversation's hostless tree is materialised for its home image: `<stagingDir>/<conversationId>`. */
  stagingDir: string
  /** The live phase of a conversation's session; null when none is live (nothing runs). */
  sessionPhase: (conversationId: string) => SessionPhase | null
}

/** What the conversation module knows about a Host it handed the agent. */
export interface HostInfo {
  conversationId: string
  target: ExecutionTarget['kind']
  /** The device behind a remote Host; null for the hostless one. */
  device: DeviceKind | null
}

/** The machine a hostless conversation moved to. */
export interface MachineTarget {
  host: RemoteHost
  home: string
}

export type SwitchTargetResult =
  | { outcome: 'switched' }
  | { outcome: 'noop' }
  | { outcome: 'conversation_not_found' }
  | { outcome: 'workspace_not_found' }
  | { outcome: 'no_hostless_entrance' }
  | { outcome: 'turn_in_flight' }
  | { outcome: 'conflict' }

/**
 * A conversation's execution target (`sessions-and-targets.md`): the one
 * place its three states are resolved to a Host and moved between — the
 * user's switch, the silent upgrade, the rule that a conversation with a
 * machine of its own never returns to hostless — keyed by the conversation,
 * whichever session (root or subagent) asks.
 */
export class ConversationTargets {
  private readonly hosts = new WeakMap<Host, HostInfo>()
  private readonly upgrades = new Map<string, Promise<MachineTarget>>()

  constructor(private readonly deps: ConversationTargetsDeps) {}

  /** The resolution order of § The three states over the conversation's record. */
  async resolve(conversationId: string): Promise<ExecutionTarget> {
    const conversation = await this.deps.control.getConversation(conversationId)
    if (!conversation) return { kind: 'hostless' }
    return resolveExecutionTarget(this.deps.control, conversation)
  }

  /**
   * The Host a conversation's actions run against: a workspace or a
   * session-bound managed host routes to the device's stable RemoteHost
   * (offline ⇒ tool errors until the runner reattaches), neither ⇒ the
   * hostless Host. A managed device is woken here — this is "the next
   * action needing the host" — after the owner check every control-plane
   * path makes.
   */
  async hostFor(conversationId: string): Promise<Host> {
    const target = await this.resolve(conversationId)
    if (target.kind === 'hostless') {
      const virtual = await this.deps.virtualHostFor(conversationId)
      this.hosts.set(virtual, { conversationId, target: 'hostless', device: null })
      return virtual
    }
    const device = await this.deps.control.getDevice(target.deviceId)
    if (device?.kind === 'managed') {
      const owner = ownerOf(device)
      const owned = target.kind === 'workspace' ? owner.kind === 'workspace' && owner.id === target.workspaceId : owner.kind === 'conversation' && owner.id === conversationId
      if (!owned) throw new ManagedHostError('not_owner', `machine ${device.id} is bound to another owner`)
      if (!this.deps.managedHosts) throw new Error('this backend provisions no machines')
      await this.deps.managedHosts.ensureRunning(device)
    }
    const path = target.kind === 'workspace' ? target.path : (this.deps.registry.deviceIdentity(target.deviceId)?.homeDir ?? '/')
    const host = this.deps.registry.hostFor({ deviceId: target.deviceId, path }, conversationId, this.deps.stores.hostStore(conversationId))
    this.hosts.set(host, { conversationId, target: target.kind, device: device?.kind ?? 'user' })
    return host
  }

  /** The conversation and target behind a Host this module handed out; null for a Host it did not. */
  hostInfo(host: Host): HostInfo | null {
    return this.hosts.get(host) ?? null
  }

  /** Whether a conversation has a machine of its own, wherever its main host is now. */
  async ownsMachine(conversationId: string): Promise<boolean> {
    return (await this.deps.control.getManagedDevice({ kind: 'conversation', id: conversationId })) !== null
  }

  /**
   * The one generic switch (§ Switching), user-initiated from the target
   * picker: refused mid-turn, compare-and-set so concurrent switches have
   * one winner, the departed device attached to the conversation at the
   * directory it was left at, the arriving device detached, the switch
   * recorded for the next turn's announcement. Files are never moved. A
   * conversation with a machine of its own has no hostless entrance, on
   * whichever target it stands.
   */
  async switch(conversationId: string, toWorkspaceId: string | null): Promise<SwitchTargetResult> {
    const { control } = this.deps
    const conversation = await control.getConversation(conversationId)
    if (!conversation) return { outcome: 'conversation_not_found' }
    if (conversation.workspaceId === toWorkspaceId && conversation.hostDeviceId === null) return { outcome: 'noop' }

    let toWorkspace: WorkspaceRecord | null = null
    if (toWorkspaceId !== null) {
      toWorkspace = await control.getWorkspace(toWorkspaceId)
      if (!toWorkspace || toWorkspace.userId !== conversation.userId) return { outcome: 'workspace_not_found' }
    } else if (conversation.hostDeviceId !== null || (await this.ownsMachine(conversationId))) {
      return { outcome: 'no_hostless_entrance' }
    }

    const phase = this.deps.sessionPhase(conversationId)
    if (phase !== null && phase !== 'idle') return { outcome: 'turn_in_flight' }

    const from = await resolveExecutionTarget(control, conversation)
    const to: ExecutionTarget = toWorkspace
      ? { kind: 'workspace', workspaceId: toWorkspace.id, deviceId: toWorkspace.deviceId, path: toWorkspace.path }
      : { kind: 'hostless' }
    const departedDeviceId = targetDeviceId(from)
    const won = await control.switchConversationTarget(
      conversationId,
      { workspaceId: conversation.workspaceId, hostDeviceId: conversation.hostDeviceId },
      { workspaceId: toWorkspaceId, hostDeviceId: null },
      { from, to },
      {
        departed: departedDeviceId === null ? null : { deviceId: departedDeviceId, cwd: from.kind === 'workspace' ? from.path : null },
        arrivingDeviceId: targetDeviceId(to),
      },
    )
    return won ? { outcome: 'switched' } : { outcome: 'conflict' }
  }

  /**
   * The session upgrade (§ Hostless execution): the conversation's files
   * become the home of a machine provisioned for it, the conversation is
   * bound to it silently, and the machine's Host is what every later action
   * resolves to. One upgrade per conversation, joined by every session that
   * asks while it runs; a failure leaves nothing behind — the conversation
   * stays hostless with its files and the next outside script tries again
   * from the tree as it stands then.
   */
  upgrade(conversationId: string): Promise<MachineTarget> {
    let pending = this.upgrades.get(conversationId)
    if (!pending) {
      pending = this.upgradeOnce(conversationId)
      this.upgrades.set(conversationId, pending)
      pending.catch(() => {
        if (this.upgrades.get(conversationId) === pending) this.upgrades.delete(conversationId)
      })
    }
    return pending
  }

  private async upgradeOnce(conversationId: string): Promise<MachineTarget> {
    const { control, managedHosts, stores } = this.deps
    if (!managedHosts) throw new Error('this script needs a machine, and this backend provisions none')
    const conversation = await control.getConversation(conversationId)
    if (!conversation) throw new Error(`no conversation ${conversationId}`)
    if (conversation.workspaceId !== null || conversation.hostDeviceId !== null) throw new Error('the conversation is no longer hostless')
    const owner = { kind: 'conversation' as const, id: conversationId }
    const home = join(this.deps.stagingDir, conversationId)
    // A tree left by an earlier attempt would merge with this one.
    await rm(home, { recursive: true, force: true })
    await stores.materializeFiles(conversationId, [
      { from: HOSTLESS_HOME, to: home },
      { from: '/tmp', to: join(home, '.tmp') },
    ])
    const device = await managedHosts.provisionFresh(owner, conversation.userId, home)
    if (!(await control.bindConversationHost(conversationId, device.id))) {
      await managedHosts.destroy(owner).catch(noop)
      await control.deleteDevice(device.id).catch(noop)
      throw new Error('the conversation left the hostless state meanwhile')
    }
    stores.clearFiles(conversationId)
    const host = await this.hostFor(conversationId)
    if (!(host instanceof RemoteHost)) throw new Error('the conversation is bound to a machine but resolves elsewhere')
    return { host, home: host.identity.homeDir }
  }
}
