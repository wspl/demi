import type { RemoteHost } from '@demicodes/host-remote'
import { reachableHosts } from './hosts'
import { ActivityGate } from '@demicodes/utils'
import type { ManagedHosts } from '../managed/lifecycle'
import type { ConversationLifecycle } from '../lifecycle/conversations'
import type { RunnerRegistry } from '../runner/registry'
import type {
  ControlService,
  ConversationTargetPointer,
  ExecutionTarget,
  DeviceRecord,
} from '../storage/control'
import type { ConversationStores } from '../storage/conversation-store'
import { administrativeStore } from '../storage/host-store'
import {
  cloudSessionDirectory,
  resolveExecutionTarget
} from './execution-target'

export interface ConversationTargetsDeps {
  control: ControlService
  registry: RunnerRegistry
  managedHosts: ManagedHosts | null
  stores: ConversationStores
  reserveTree: (conversationId: string) => (() => void) | null
  lifecycle?: ConversationLifecycle
}
export type SwitchTargetResult = {
  outcome:
    'switched' |
    'noop' |
    'conversation_not_found' |
    'workspace_not_found' |
    'device_not_found' |
    'archived' |
    'turn_in_flight' |
    'conflict'
}

export class HostAccessRefused extends Error {
  constructor(readonly code: 'conversation_archived' | 'host_not_attached', message: string) {
    super(message)
  }
}

export type ConversationHostAccess = ConversationTargets['withHost']

interface HostSelection {
  device: DeviceRecord
  path: string
  prepareDirectory: boolean
}

export class ConversationTargets {
  /** Device access is conversation-less; its Hosts share one scratch scope. */
  private readonly fileActivity = new Map<string, ActivityGate>()
  private readonly activityObservers = new Set<(id: string, active: boolean) => void>()
  private readonly subscriptions = new DisposableStack()
  constructor(private readonly deps: ConversationTargetsDeps) {}

  files(id: string): ActivityGate {
    let gate = this.fileActivity.get(id)
    if (!gate) {
      gate = new ActivityGate()
      this.fileActivity.set(id, gate)
      const observed = gate
      this.subscriptions.defer(gate.subscribe(() => {
        for (const changed of this.activityObservers) changed(id, observed.demandActive)
      }))
    }
    return gate
  }

  observeActivity(changed: (id: string, active: boolean) => void): () => void {
    this.activityObservers.add(changed)
    return () => { this.activityObservers.delete(changed) }
  }

  async releaseConversation(id: string, deviceId?: string): Promise<void> {
    await this.deps.lifecycle?.release(id, deviceId)
  }

  async detach(id: string, deviceId: string): Promise<boolean> {
    const tree = this.deps.reserveTree(id)
    if (!tree) return false
    const file = this.files(id).tryReserve()
    if (!file) {
      tree()
      return false
    }
    try {
      const attached = await this.deps.control.listAttachedHosts(id)
      if (attached.some(host => host.deviceId === deviceId)) {
        await this.releaseConversation(id, deviceId)
        await this.deps.control.detachHost(id, deviceId)
      }
      return true
    } finally {
      file()
      tree()
    }
  }

  close(): void {
    this.subscriptions.dispose()
    this.activityObservers.clear()
  }

  /**
   * The one way to reach a conversation's main or attached host from outside the
   * agent: every operation that touches the host on the conversation's
   * behalf (writing an attachment, listing the working tree, reading a
   * file) runs through here, whatever the target is. It resolves the target
   * as the agent does, refuses an archived conversation, wakes a stopped
   * Cloud and holds it for the operation, and takes the conversation's file
   * gate so the operation excludes an archive or a target switch. A paired
   * device without a live runner fails inside the operation as the runner's
   * offline error. Nothing reaches the host around this method.
   * Lifecycle access instead skips offline and Cloud devices, takes no file
   * gate, and neither prepares directories nor wakes a machine.
   */
  async withHost<T>(
    id: string,
    operation: (host: RemoteHost) => Promise<T>,
    options: { lifecycle: true; deviceId?: string },
  ): Promise<T | undefined>
  async withHost<T>(
    id: string,
    operation: (host: RemoteHost) => Promise<T>,
    options?: { signal?: AbortSignal; deviceId?: string },
  ): Promise<T>
  async withHost<T>(
    id: string,
    operation: (host: RemoteHost) => Promise<T>,
    options: { signal?: AbortSignal; deviceId?: string; lifecycle?: boolean } = {},
  ): Promise<T | undefined> {
    const { signal, deviceId: selectedDeviceId } = options
    if (options.lifecycle) {
      const target = await this.resolve(id)
      if (!selectedDeviceId && !target.deviceId) return
      const selected = await this.select(id, selectedDeviceId, false)
      if (selected.device.kind === 'managed') return
      if (!this.deps.registry.deviceOnline(selected.device.id)) return
      const host = this.deps.registry.hostFor({ deviceId: selected.device.id, path: selected.path }, id, this.deps.stores.hostStore(id))
      return await operation(host)
    }
    let machine: { deviceId: string; release: () => void } | undefined
    try {
      while (true) {
        const releaseFiles = await this.files(id).enter(signal)
        let selected: HostSelection
        try {
          selected = await this.select(id, selectedDeviceId, true)
          if (selected.device.kind !== 'managed' || machine?.deviceId === selected.device.id) {
            if (selected.device.kind !== 'managed') {
              machine?.release()
              machine = undefined
            }
            const host = await this.openHost(id, selected)
            this.deps.lifecycle?.track(id)
            return await operation(host)
          }
        } finally {
          releaseFiles()
        }
        // Waiting for a device transition holds no conversation file reservation.
        // Recheck archive, ownership and target selection after admission succeeds.
        machine?.release()
        machine = undefined
        const release = await this.deps.managedHosts!.enter(selected.device, signal)
        machine = { deviceId: selected.device.id, release }
      }
    } finally {
      machine?.release()
    }
  }

  async resolve(id: string): Promise<ExecutionTarget> {
    const conversation = await this.deps.control.getConversation(id)
    if (!conversation)
      throw new Error(`No conversation ${id}`)
    return resolveExecutionTarget(
      this.deps.control,
      this.deps.registry,
      conversation
    )
  }

  async hostFor(id: string, selectedDeviceId?: string): Promise<RemoteHost> {
    return this.withHost(id, async host => host, { deviceId: selectedDeviceId })
  }

  /**
   * Device access — the one entry `withHost` has no conversation for
   * (`sessions-and-targets.md` § Host operations): the registry's Host for a
   * device the owner owns, admitted only while connected. It takes no file
   * gate (it touches no conversation files) and never wakes a stopped Cloud
   * (a stop has already destroyed the device's exposes). It exists for the
   * public relay alone; its calls mark no machine activity.
   */
  async deviceHost(userId: string, deviceId: string): Promise<RemoteHost | null> {
    const device = await this.deps.control.getDevice(deviceId)
    if (!device || device.userId !== userId)
      return null
    if (!this.deps.registry.deviceOnline(deviceId))
      return null
    return this.deps.registry.hostFor(
      { deviceId, path: '/' },
      DEVICE_ACCESS_CONVERSATION,
      administrativeStore,
      { admit: false }
    )
  }

  /** Resolve the authorized Host binding without waiting for any device transition. */
  private async select(id: string, selectedDeviceId: string | undefined, allocate: boolean): Promise<HostSelection> {
    const { control, registry, managedHosts } = this.deps
    const conversation = await control.getConversation(id)
    if (!conversation) throw new Error(`No conversation ${id}`)
    if (conversation.archived)
      throw new HostAccessRefused('conversation_archived', 'Conversation is archived')
    const target = await this.resolve(id)
    const selected = selectedDeviceId === undefined
      ? undefined
      : (await reachableHosts(this.deps, id)).find(host => host.deviceId === selectedDeviceId)
    if (selectedDeviceId !== undefined && !selected)
      throw new HostAccessRefused('host_not_attached', 'No such host in this conversation')
    const device = selected
      ? await control.getDevice(selected.deviceId)
      : target.kind === 'cloud'
        ? allocate ? await control.getOrCreateCloudDevice(conversation.userId) : await control.getManagedDevice(conversation.userId)
        : await control.getDevice(target.deviceId)
    if (!device || device.userId !== conversation.userId)
      throw new Error('Device not owned by this user')
    if (device.kind === 'managed' && !managedHosts)
      throw new Error('Cloud is not configured')
    const path = selected?.role === 'attached'
      ? selected.path || registry.deviceIdentity(device.id)?.homeDir || ''
      : target.kind === 'cloud'
        ? (conversation.target.kind === 'cloud' ? conversation.target.path : undefined)
          ?? cloudSessionDirectory(id, registry.deviceIdentity(device.id)?.homeDir)
        : target.path
    return { device, path, prepareDirectory: selected?.role !== 'attached' && target.kind === 'cloud' }
  }

  /** Host preparation occurs only after ordinary demand has obtained device admission. */
  private async openHost(id: string, selected: HostSelection): Promise<RemoteHost> {
    const host = this.deps.registry.hostFor({ deviceId: selected.device.id, path: selected.path }, id, this.deps.stores.hostStore(id))
    if (selected.prepareDirectory)
      await host.fs.mkdir(selected.path, { recursive: true })
    return host
  }

  async switch(
    id: string,
    to: ConversationTargetPointer
  ): Promise<SwitchTargetResult> {
    const { control } = this.deps
    const conversation = await control.getConversation(id)
    if (!conversation)
      return { outcome: 'conversation_not_found' }
    if (conversation.archived)
      return { outcome: 'archived' }
    if (JSON.stringify(conversation.target) === JSON.stringify(to))
      return { outcome: 'noop' }
    if (to.kind === 'workspace') {
      const workspace = await control.getWorkspace(to.workspaceId)
      if (!workspace || workspace.userId !== conversation.userId)
        return { outcome: 'workspace_not_found' }
    }
    if (to.kind === 'device') {
      const device = await control.getDevice(to.deviceId)
      if (!device ||
        device.userId !== conversation.userId ||
        device.kind !== 'user')
        return { outcome: 'device_not_found' }
    }
    const releaseTree = this.deps.reserveTree(id)
    if (!releaseTree)
      return { outcome: 'turn_in_flight' }
    const releaseFiles = this.files(id).tryReserve()
    if (!releaseFiles) {
      releaseTree()
      return { outcome: 'turn_in_flight' }
    }
    try {
      if ((await control.getConversation(id))?.archived)
        return { outcome: 'archived' }
      const from = await resolveExecutionTarget(
        control,
        this.deps.registry,
        conversation
      )
      const destination = await resolveExecutionTarget(
        control,
        this.deps.registry,
        { ...conversation, target: to }
      )
      const deviceId = from.deviceId
      if (deviceId && deviceId !== destination.deviceId) await this.releaseConversation(id, deviceId)
      const won = await control.switchConversationTarget(
        id,
        conversation.target,
        to,
        { from, to: destination },
        {
          departed: deviceId === null ? null : { deviceId, cwd: from.path },
          arrivingDeviceId: destination.deviceId,
        }
      )
      if (won) {
        this.deps.lifecycle?.reset(id)
      }
      return { outcome: won ? 'switched' : 'conflict' }
    } finally {
      releaseFiles()
      releaseTree()
    }
  }
}

const DEVICE_ACCESS_CONVERSATION = '\0device-access'
