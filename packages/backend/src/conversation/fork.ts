import type { ModelSelection } from '@demicodes/core'
import { ForkPreparationError, type AgentServer } from '@demicodes/agent'
import type { ControlService, ConversationRecord } from '../storage/control'
import type { ConversationStores } from '../storage/conversation-store'
import type { RunnerRegistry } from '../runner/registry'
import { resolveExecutionTarget } from './execution-target'

export class ForkRefused extends Error {
  constructor(readonly status: 400 | 404 | 409, readonly code: string, message: string) {
    super(message)
  }
}

/** A reserved UUID is published only after its independent root commits. */
export class ConversationForks {
  private readonly pending = new Map<string, Promise<unknown>>()

  constructor(private readonly deps: {
    control: ControlService
    stores: ConversationStores
    server: AgentServer
    registry: Pick<RunnerRegistry, 'deviceIdentity'>
  }) {}

  async recover(): Promise<void> {
    for (const operation of await this.deps.control.pendingConversationForks()) {
      if (await this.deps.stores.treeStore(operation.id).node(operation.id)) {
        await this.deps.control.publishConversationFork(operation.id)
      }
    }
  }

  async create(userId: string, sourceId: string, id: string, blockId: string): Promise<{
    conversation: ConversationRecord
    created: boolean
    model: ModelSelection
  }> {
    const previous = this.pending.get(id) ?? Promise.resolve()
    // Earlier callers receive their own failure; it must not poison the retry queue.
    const run = previous.catch(() => {}).then(() => this.createReserved(userId, sourceId, id, blockId))
    this.pending.set(id, run)
    try {
      return await run
    } finally {
      if (this.pending.get(id) === run) {
        this.pending.delete(id)
      }
    }
  }

  private async createReserved(userId: string, sourceId: string, id: string, blockId: string) {
    const { control, stores, server, registry } = this.deps
    const source = await control.getConversation(sourceId)
    if (!source || source.userId !== userId) {
      throw new ForkRefused(404, 'conversation_not_found', 'No such conversation')
    }
    let operation = await control.getConversationFork(id)
    if (operation && (operation.userId !== userId || operation.sourceId !== sourceId || operation.blockId !== blockId)) {
      throw new ForkRefused(409, 'fork_conflict', 'The Fork UUID belongs to another creation attempt')
    }
    const existing = await control.getConversation(id)
    if (existing) {
      if (!operation) {
        throw new ForkRefused(409, 'id_unavailable', 'Conversation id is unavailable')
      }
      return { conversation: existing, created: false, model: operation.model }
    }
    if (operation && await stores.treeStore(id).node(id)) {
      return { conversation: await control.publishConversationFork(id), created: false, model: operation.model }
    }

    let checkpoint
    try {
      checkpoint = await server.prepareFork(sourceId, blockId)
    } catch (error) {
      if (error instanceof ForkPreparationError) {
        throw new ForkRefused(400, 'invalid_fork_target', error.message)
      }
      throw error
    }
    if (!operation) {
      const target = await resolveExecutionTarget(control, registry, source)
      operation = await control.reserveConversationFork({
        id, userId, sourceId, blockId,
        title: `${source.title} (Fork)`,
        target: source.target.kind === 'cloud' ? { kind: 'cloud', path: target.path } : source.target,
        model: checkpoint.model,
        createdAt: new Date().toISOString(),
        attachedHosts: (await control.listAttachedHosts(sourceId)).map((host) => ({
          deviceId: host.deviceId, name: host.name, cwd: host.cwd,
        })),
      })
      if (!operation) {
        throw new ForkRefused(409, 'id_unavailable', 'Conversation id is unavailable')
      }
    }
    checkpoint.model = structuredClone(operation.model)
    await server.initializeFork(id, checkpoint)
    return { conversation: await control.publishConversationFork(id), created: true, model: operation.model }
  }
}
