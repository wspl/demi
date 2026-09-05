import type { Block, QueuedMessage } from '@demicodes/core'
import { completedChildrenCarriedBy } from './store/tree-store'
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
  state: Omit<AgentSessionCheckpoint<State>, 'transcript'>
  blocks: Map<number, Block>
  blockCount: number
}

/**
 * The in-memory `AgentTreeStore`: the contract's semantics with nothing
 * durable, for tests and fixtures. One instance may hold any number of
 * roots. `saves` records every journal write for assertions.
 */
export class MemoryAgentStore<State = unknown> implements AgentTreeStore<State> {
  readonly nodes = new Map<string, StoredNode<State>>()
  readonly saves: Array<{ id: string; update: AgentSessionPersistUpdate<State> }> = []

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

  async createNode(record: AgentNodeRecord, checkpoint: AgentSessionPersistUpdate<State>): Promise<void> {
    if (this.nodes.has(record.id)) throw new Error(`MemoryAgentStore: node "${record.id}" already exists`)
    const stored: StoredNode<State> = { record: structuredClone(record), state: snapshotOf(checkpoint), blocks: new Map(), blockCount: 0 }
    this.nodes.set(record.id, stored)
    this.applySave(record.id, stored, checkpoint)
  }

  sessionStore(id: string): AgentSessionStore<State> {
    return {
      save: (update) => {
        this.applySave(id, this.require(id), update)
      },
      load: async () => this.load(id),
    }
  }

  async closeNode(id: string, close: AgentNodeClose): Promise<void> {
    const stored = this.require(id)
    stored.record = { ...stored.record, closedPhase: close.phase, closedAt: close.closedAt, result: close.result, failure: close.failure, delivered: false }
  }

  async reopenNode(id: string, fields: { metadata: AgentMetadata | null; spawnedAt: number }, message: QueuedMessage): Promise<void> {
    const stored = this.require(id)
    stored.record = { ...stored.record, ...structuredClone(fields), closedPhase: null, closedAt: null, result: null, failure: null, delivered: false }
    stored.state = { ...stored.state, queue: [structuredClone(message)] }
  }

  async markDelivered(id: string): Promise<void> {
    this.require(id).record.delivered = true
  }

  async deleteNode(id: string): Promise<void> {
    for (const child of await this.children(id)) await this.deleteNode(child.id)
    this.nodes.delete(id)
  }

  private applySave(id: string, stored: StoredNode<State>, update: AgentSessionPersistUpdate<State>): void {
    this.saves.push({ id, update: structuredClone(update) })
    for (const { index, block } of update.changedBlocks) stored.blocks.set(index, structuredClone(block))
    for (const index of [...stored.blocks.keys()]) {
      if (index >= update.blockCount) stored.blocks.delete(index)
    }
    stored.blockCount = update.blockCount
    stored.state = snapshotOf(update)
    for (const childId of completedChildrenCarriedBy(update)) {
      const child = this.nodes.get(childId)
      if (child && child.record.parentId === id) child.record.delivered = true
    }
  }

  private load(id: string): AgentSessionCheckpoint<State> | null {
    const stored = this.nodes.get(id)
    if (!stored) return null
    const blocks: Block[] = []
    for (let index = 0; index < stored.blockCount; index += 1) {
      const block = stored.blocks.get(index)
      if (!block) throw new Error(`MemoryAgentStore: node "${id}" is missing block row ${index}`)
      blocks.push(structuredClone(block))
    }
    return { ...structuredClone(stored.state), transcript: { blocks } }
  }

  private require(id: string): StoredNode<State> {
    const stored = this.nodes.get(id)
    if (!stored) throw new Error(`MemoryAgentStore: no node "${id}"`)
    return stored
  }
}

function snapshotOf<State>(update: AgentSessionPersistUpdate<State>): Omit<AgentSessionCheckpoint<State>, 'transcript'> {
  return {
    state: structuredClone(update.state),
    phase: update.phase,
    queue: structuredClone(update.queue),
    cwd: update.cwd,
    model: structuredClone(update.model),
    harnessName: update.harnessName,
  }
}

/** A store factory for one server: every root the server opens shares one in-memory store. */
export function memoryAgentStores(): (rootSessionId: string) => MemoryAgentStore {
  const store = new MemoryAgentStore()
  return () => store
}
