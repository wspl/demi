import type { Host } from '@demicodes/shell'
import { ActivityGate } from '@demicodes/utils'
import type { ManagedHosts } from '../managed/lifecycle'
import type { RunnerRegistry } from '../runner/registry'
import type {
  ControlService,
  ConversationTargetPointer,
  ExecutionTarget
} from '../storage/control'
import type { ConversationStores } from '../storage/conversation-store'
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

export class ConversationTargets {
  private readonly fileActivity = new Map<string, ActivityGate>()
  constructor(private readonly deps: ConversationTargetsDeps) {}

  files(id: string): ActivityGate {
    let gate = this.fileActivity.get(id)
    if (!gate) {
      gate = new ActivityGate()
      this.fileActivity.set(id, gate)
    }
    return gate
  }

  async withHost<T>(
    id: string,
    operation: (host: Host) => Promise<T>,
    signal?: AbortSignal
  ): Promise<T> {
    const release = await this.files(id).enter(signal)
    let releaseMachine: (() => void) | undefined
    try {
      if ((await this.deps.control.getConversation(id))?.archived)
        throw new Error('Conversation is archived')
      const host = await this.hostFor(id)
      const deviceId = (await this.resolve(id)).deviceId
      const device = deviceId
        ? await this.deps.control.getDevice(deviceId)
        : null
      if (device?.kind === 'managed')
        releaseMachine = await this.deps.managedHosts!.enter(device, signal)
      return await operation(host)
    } finally {
      releaseMachine?.()
      release()
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

  async hostFor(id: string): Promise<Host> {
    const { control, registry, managedHosts, stores } = this.deps
    const conversation = await control.getConversation(id)
    if (!conversation)
      throw new Error(`No conversation ${id}`)
    const target = await this.resolve(id)
    const device = target.kind === 'cloud'
      ? await control.getOrCreateCloudDevice(conversation.userId)
      : await control.getDevice(target.deviceId)
    if (!device || device.userId !== conversation.userId)
      throw new Error('Device not owned by this user')
    if (device.kind === 'managed') {
      if (!managedHosts)
        throw new Error('Cloud is not configured')
      await managedHosts.ensureRunning(device)
    }
    const path = target.kind === 'cloud'
      ? (conversation.target.kind === 'cloud' ? conversation.target.path : undefined)
        ?? cloudSessionDirectory(id, registry.deviceIdentity(device.id)?.homeDir)
      : target.path
    const host = registry.hostFor(
      { deviceId: device.id, path },
      id,
      stores.hostStore(id)
    )
    if (target.kind === 'cloud')
      await host.fs.mkdir(path, { recursive: true })
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
      return { outcome: won ? 'switched' : 'conflict' }
    } finally {
      releaseFiles()
      releaseTree()
    }
  }
}
