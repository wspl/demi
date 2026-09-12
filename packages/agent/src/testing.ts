import { stringifyPortableJson } from '@demicodes/utils'
import type { Block, QueuedMessage } from '@demicodes/core'
import { completedChildrenCarriedBy } from './store/tree-store'
import { commandStateSchema, emptyCommandState, type CommandStateSnapshot } from './store/command-state'
import type {
  AgentMetadata,
  AgentNodeClose,
  AgentNodeRecord,
  AgentSessionCheckpoint,
  AgentSessionPersistUpdate,
  AgentSessionStore,
  AgentTreeStore,
} from './types'

interface StoredNode<State> {
  record: AgentNodeRecord
  state: Omit<AgentSessionCheckpoint<State>, 'transcript' | 'commandState'>
  commandState: CommandStateSnapshot
  blocks: Map<number, Block>
  blockCount: number
}

/**
 * The in-memory `AgentTreeStore`: the contract's semantics with nothing
 * durable, for tests and fixtures. One instance may hold any number of
 * roots. `saves` records every journal write for assertions.
 */
export class MemoryAgentStore<State = unknown>
  implements AgentTreeStore<State> {
  readonly nodes = new Map<string, StoredNode<State>>()
  readonly saves: Array<{
    id: string;
    update: AgentSessionPersistUpdate<State>
  }> = []

  async node(id: string): Promise<AgentNodeRecord | null> {
    const stored = this.nodes.get(id)
    return stored ? structuredClone(stored.record) : null
  }

  async children(parentId: string): Promise<AgentNodeRecord[]> {
    return [...this.nodes.values()]
      .filter((stored) => stored.record.parentId === parentId)
      .sort((a, b) => a.record.spawnedAt - b.record.spawnedAt)
      .map((stored) => structuredClone(stored.record))
  }

  async createNode(
    record: AgentNodeRecord,
    checkpoint: AgentSessionPersistUpdate<State>
  ): Promise<void> {
    if (this.nodes.has(record.id))
      throw new Error(`MemoryAgentStore: node "${record.id}" already exists`)
    const stored: StoredNode<State> = {
      record: structuredClone(record),
      state: snapshotOf(checkpoint),
      commandState: commandStateSchema.parse(checkpoint.commandState ?? emptyCommandState()),
      blocks: new Map(),
      blockCount: 0
    }
    this.nodes.set(record.id, stored)
    this.applySave(record.id, stored, checkpoint)
  }

  sessionStore(id: string): AgentSessionStore<State> {
    return {
      save: (update, options) => {
        options?.signal?.throwIfAborted()
        this.applySave(id, this.require(id), update)
      },
      load: async () => this.load(id),
    }
  }

  async closeNode(id: string, close: AgentNodeClose): Promise<void> {
    const stored = this.require(id)
    stored.record = {
      ...stored.record,
      closedPhase: close.phase,
      closedAt: close.closedAt,
      result: close.result,
      failure: close.failure,
      delivered: false
    }
  }

  async reopenNode(
    id: string,
    fields: {
      metadata: AgentMetadata | null;
      spawnedAt: number
    },
    message: QueuedMessage
  ): Promise<void> {
    const stored = this.require(id)
    stored.record = {
      ...stored.record,
      ...structuredClone(fields),
      closedPhase: null,
      closedAt: null,
      result: null,
      failure: null,
      delivered: false
    }
    stored.state = { ...stored.state, queue: [structuredClone(message)] }
  }

  async markDelivered(id: string, spawnedAt: number): Promise<void> {
    const record = this.require(id).record
    if (record.spawnedAt === spawnedAt) {
      record.delivered = true
    }
  }

  async deleteNode(id: string): Promise<void> {
    for (const child of await this.children(id)) await this.deleteNode(child.id)
    this.nodes.delete(id)
  }

  private applySave(
    id: string,
    stored: StoredNode<State>,
    update: AgentSessionPersistUpdate<State>
  ): void {
    const completedRounds = completedChildrenCarriedBy(update)
    const commandState = update.commandState ? commandStateSchema.parse(update.commandState) : stored.commandState
    if (update.commandState) {
      const previous = new Map(stored.commandState.versions.map((version) => [version.revision, version]))
      for (const version of commandState.versions) {
        const existing = previous.get(version.revision)
        if (existing && stringifyPortableJson(existing.values) !== stringifyPortableJson(version.values)) {
          throw new Error(`Command-state version ${version.revision} is immutable`)
        }
      }
    }
    this.saves.push({ id, update: structuredClone(update) })
    for (const { index, block } of update.changedBlocks) stored.blocks.set(
      index,
      structuredClone(block)
    )
    for (const index of [...stored.blocks.keys()]) {
      if (index >= update.blockCount)
        stored.blocks.delete(index)
    }
    stored.blockCount = update.blockCount
    stored.state = snapshotOf(update)
    stored.commandState = structuredClone(commandState)
    for (const round of completedRounds) {
      const child = this.nodes.get(round.id)
      if (child && child.record.parentId === id && child.record.spawnedAt === round.spawnedAt)
        child.record.delivered = true
    }
  }

  private load(id: string): AgentSessionCheckpoint<State> | null {
    const stored = this.nodes.get(id)
    if (!stored)
      return null
    const blocks: Block[] = []
    for (let index = 0; index < stored.blockCount; index += 1) {
      const block = stored.blocks.get(index)
      if (!block)
        throw new Error(
          `MemoryAgentStore: node "${id}" is missing block row ${index}`
        )
      blocks.push(structuredClone(block))
    }
    return { ...structuredClone(stored.state), commandState: structuredClone(stored.commandState), transcript: { blocks } }
  }

  private require(id: string): StoredNode<State> {
    const stored = this.nodes.get(id)
    if (!stored)
      throw new Error(`MemoryAgentStore: no node "${id}"`)
    return stored
  }
}

function snapshotOf<State>(
  update: AgentSessionPersistUpdate<State>
): Omit<AgentSessionCheckpoint<State>, 'transcript' | 'commandState'> {
  return {
    state: structuredClone(update.state),
    phase: update.phase,
    queue: structuredClone(update.queue),
    pendingInternalSteers: structuredClone(update.pendingInternalSteers),
    cwd: update.cwd,
    model: structuredClone(update.model),
    harnessName: update.harnessName,
    ...(update.edits ? { edits: structuredClone(update.edits) } : {}),
  }
}

/**
 * A store factory for one server: every root the server opens shares one
 * in-memory store.
 */
export function memoryAgentStores(): (rootSessionId: string) => MemoryAgentStore {
  const store = new MemoryAgentStore()
  return () => store
}
