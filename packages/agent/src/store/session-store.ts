import type { Block } from '@demicodes/core'
import type { HostStore } from '@demicodes/shell'
import { externalizeBlockMedia, rehydrateBlockMedia, type BlobStore } from './media'
import type {
  AgentSessionCheckpoint,
  AgentSessionPersistUpdate,
  AgentSessionStateSnapshot,
  AgentSessionStore,
} from '../types'


/** The persisted state row: the non-transcript snapshot plus the row count. */
export interface PersistedSessionState<State> extends AgentSessionStateSnapshot<State> {
  blockCount: number
}

const STATE_KEY = 'state.json'
const BLOCKS_PREFIX = 'blocks/'
// Fixed-width decimal so HostStore's lexicographic list order is index order.
const INDEX_WIDTH = 8

export interface HostAgentSessionStoreOptions {
  /** When present, media bytes are externalized to it on save and rehydrated on load. */
  blobs?: BlobStore
}

/**
 * The `AgentSessionStore` realization over a `HostStore`: one entry per
 * transcript block (`<prefix>/blocks/<index>.json`) plus one state entry
 * (`<prefix>/state.json`). Used wherever sessions persist through a Host —
 * the agent server and subagent children alike.
 */
export function hostAgentSessionStore<State>(
  store: HostStore,
  prefix: string,
  options: HostAgentSessionStoreOptions = {},
): AgentSessionStore<State> {
  const stateKey = `${prefix}/${STATE_KEY}`
  const blockKey = (index: number): string => `${prefix}/${BLOCKS_PREFIX}${String(index).padStart(INDEX_WIDTH, '0')}.json`

  return {
    async save(update: AgentSessionPersistUpdate<State>): Promise<void> {
      for (const { index, block } of update.changedBlocks) {
        const persisted = options.blobs ? await externalizeBlockMedia(block, options.blobs) : block
        await store.writeJson(blockKey(index), persisted)
      }
      const state: PersistedSessionState<State> = {
        blockCount: update.blockCount,
        state: update.state,
        phase: update.phase,
        queue: update.queue,
        cwd: update.cwd,
        model: update.model,
        harnessName: update.harnessName,
      }
      await store.writeJson(stateKey, state)
      for (const key of await store.list(`${prefix}/${BLOCKS_PREFIX}`)) {
        const index = parseBlockIndex(key)
        if (index !== null && index >= update.blockCount) await store.delete(key)
      }
    },

    async load(): Promise<AgentSessionCheckpoint<State> | null> {
      const loaded = await loadPersistedSession<State>(store, prefix)
      if (!loaded) return null
      const blocks = options.blobs
        ? await Promise.all(loaded.blocks.map((block) => rehydrateBlockMedia(block, options.blobs!)))
        : loaded.blocks
      return { ...toCheckpointFields(loaded.state), transcript: { blocks } }
    },
  }
}

/**
 * Reads the persisted rows without media rehydration — the shape summaries
 * and cold reads need. Returns null when no state entry exists.
 */
export async function loadPersistedSession<State>(
  store: HostStore,
  prefix: string,
): Promise<{ state: PersistedSessionState<State>; blocks: Block[] } | null> {
  const state = await store.readJson<PersistedSessionState<State>>(`${prefix}/${STATE_KEY}`)
  if (!state) return null
  const keys = (await store.list(`${prefix}/${BLOCKS_PREFIX}`)).sort()
  const rows: Block[] = []
  for (const key of keys) {
    const index = parseBlockIndex(key)
    if (index === null || index !== rows.length || index >= state.blockCount) break
    const block = await store.readJson<Block>(key)
    if (!block) break
    rows.push(block)
  }
  return { state, blocks: rows }
}

/** Assembles a checkpoint from a raw persisted read (no media rehydration). */
export function persistedSessionCheckpoint<State>(loaded: {
  state: PersistedSessionState<State>
  blocks: Block[]
}): AgentSessionCheckpoint<State> {
  return { ...toCheckpointFields(loaded.state), transcript: { blocks: loaded.blocks } }
}

function toCheckpointFields<State>(state: PersistedSessionState<State>): Omit<AgentSessionCheckpoint<State>, 'transcript'> {
  return {
    state: state.state,
    phase: state.phase,
    queue: state.queue,
    cwd: state.cwd,
    model: state.model,
    harnessName: state.harnessName,
  }
}

function parseBlockIndex(key: string): number | null {
  const name = key.slice(key.lastIndexOf('/') + 1)
  const match = /^(\d+)\.json$/.exec(name)
  return match ? Number.parseInt(match[1], 10) : null
}

