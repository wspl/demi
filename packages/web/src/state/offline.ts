import { toRaw, watch } from 'vue'
import { z } from 'zod'
import {
  agentEntrySchema,
  blockSchema,
  conversationAttachmentSchema,
  instructionEntrySchema,
  PAGE_BLOCKS,
  providerFailureFactsSchema,
  schemaRefusal,
  versionSchema,
  versionsSchema,
  type ProviderFailureFacts,
  type Version,
} from '@demicodes/protocol'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { heldTranscript, type HeldBlock, type HeldTranscript, type TranscriptWindow } from '@demicodes/web-ui/transport/protocol'
import { onSessionExpired } from '../api/client'
import {
  conversationMetadataSchema,
  productStateSchema,
  userDtoSchema,
  type ProductState,
  type UserDto,
} from '../api/generated/web-api'
import { IndexedDatabase } from '@demicodes/web-ui/infra/indexed-db'
import { t } from './locales/text'

/**
 * The account's disk copy (`docs/decisions/0009-agent-sessions-and-sync.md`): what
 * the page has read, so a reload, a return and a tab opened again show it at
 * once and catch up by changes alone, and so the page opens offline.
 *
 * - The account's record: the product state, with the conversation list,
 *   and the account the session names.
 * - Each conversation's record: its metadata and its agents at their
 *   version, and the descriptions of the attachments its replies name, so
 *   an opening from the copy shows the whole conversation at once
 *   (`docs/decisions/0029-a-conversation-opens-whole.md`).
 * - Each node's record and blocks: the runs of its transcript the page knows
 *   to be whole, its length, and the version its latest run is current at;
 *   each block by node and index, read one window at a time.
 *
 * A block is written once it is complete, never per streamed piece. The
 * disk is bounded at `DISK_BYTES`, or what the browser grants if less: past
 * it the node viewed longest ago goes first, its blocks deleted a small
 * batch at a time while the page is idle, since one large deletion holds
 * every read back until it ends; a write the disk is too full for evicts
 * rather than fails.
 *
 * It is disposable: written by one build and read by that build alone, a
 * record of another build is dropped, never migrated, and it is the
 * account's alone.
 */

/** The build the records belong to; Vite's development server names none. */
function cacheBuild(): string {
  return import.meta.env.DEMI_WEB_BUILD ?? 'development'
}

const ACCOUNT = 'account'
const CONVERSATIONS = 'conversations'
/**
 * Each node's record by its id, with when it was last viewed, by whose
 * index the node viewed longest ago is found, and its conversation's id,
 * by whose index a conversation's nodes go with it.
 */
const NODES = 'nodes'
/** Each block by `[node, index]`. */
const BLOCKS = 'blocks'
/** The one key of the account's record. */
const ACCOUNT_KEY = 'account'

/** The most the disk holds, in bytes, when the browser grants as much. */
export const DISK_BYTES = 500 * 1024 * 1024
/** The blocks one idle step deletes at most, so no deletion holds reads back for long: a number of its own, which the page's size does not set. */
export const EVICTION_BATCH = 200
/** The blocks of a node's run a view reads from the disk at once, a page. */
export const DISK_WINDOW = PAGE_BLOCKS

/**
 * The database's layout. Its records are disposable, so a change of their
 * shape needs no new version: a record of another build is dropped. A new
 * layout drops every store of the one before, records and all, as a cache
 * of another build is dropped.
 */
const database = new IndexedDatabase('demi-offline', 3, (request) => {
  const db = request.result
  for (const name of [...db.objectStoreNames]) {
    db.deleteObjectStore(name)
  }
  db.createObjectStore(ACCOUNT)
  db.createObjectStore(CONVERSATIONS)
  const nodes = db.createObjectStore(NODES)
  nodes.createIndex('viewedAt', 'viewedAt')
  nodes.createIndex('conversation', 'conversation')
  db.createObjectStore(BLOCKS)
})

const accountRecordSchema = z.object({
  build: z.string(),
  user: userDtoSchema,
  state: productStateSchema,
})

const conversationRecordSchema = z.object({
  build: z.string(),
  userId: z.string(),
  metadata: conversationMetadataSchema,
  agents: z.object({ version: versionSchema, entries: z.array(agentEntrySchema) }),
  /** The descriptions of the attachments its replies name, by node and id. */
  attachments: z.record(z.string(), z.record(z.string(), conversationAttachmentSchema)),
})

/** A run of blocks the disk holds whole, from `start` to before `end`. */
const runSchema = z.object({ start: z.int().nonnegative(), end: z.int().nonnegative() })

const nodeRecordSchema = z.object({
  build: z.string(),
  userId: z.string(),
  conversation: z.string(),
  /** How many blocks its transcript held, as far as the latest run knows. */
  length: z.int().nonnegative(),
  /** The version its latest run is current at; absent when that run lacks blocks still streaming. */
  version: versionSchema.optional(),
  runs: z.array(runSchema),
  /** The root's newest instructions and its compaction summaries, as pages named them. */
  instructions: z.array(instructionEntrySchema),
  summaries: z.record(z.string(), z.int().nonnegative()),
  viewedAt: z.number(),
  /** About how many bytes its blocks take, counted as they are written. */
  bytes: z.number().nonnegative(),
})

/** A block as the page held it: whole, or light when it was read from a page, with its failure's facts. */
const blockRecordSchema = z.object({
  block: z.intersection(
    blockSchema,
    z.object({ light: z.literal(true).optional(), versions: versionsSchema.optional() }),
  ),
  failure: providerFailureFactsSchema.optional(),
})

export type ConversationCopy = Omit<z.infer<typeof conversationRecordSchema>, 'build' | 'userId'>
type NodeRecord = z.infer<typeof nodeRecordSchema>
type Run = z.infer<typeof runSchema>

/** What the disk holds of a node, as a view shows it at once. */
export interface NodeCopy {
  transcript: HeldTranscript
  failures: Record<string, ProviderFailureFacts>
  instructions: NodeRecord['instructions']
  summaries: NodeRecord['summaries']
}

/** What a page writes of a node: its runs as they are now, and the blocks that changed in them. */
export interface NodeWrite {
  conversation: string
  length: number
  version?: Version
  runs: Run[]
  instructions: NodeRecord['instructions']
  summaries: NodeRecord['summaries']
  /** The blocks to write, by index, each with its failure's facts. */
  blocks: { index: number, block: HeldBlock, failure?: ProviderFailureFacts }[]
  /** The index from which the disk's blocks go, as a rewrite that cut the transcript there; none for none. */
  cutFrom?: number
  /**
   * Whether `runs` replace the disk's, as after a reset that could not vouch
   * for what the page held, which the disk holds as well; else they join them.
   */
  replaceRuns?: true
}

export interface AccountCopy {
  user: UserDto
  state: ProductState
}


/**
 * `value` as the database keeps it: the plain data under the reactive
 * proxies that hold it, which the database's copy of a record cannot read.
 * Only an array or object that holds a proxy is copied, with the plain
 * data in its place; the rest is the data itself. The database copies the
 * record itself, as the browser's own structured clone. A conversation's
 * copy needs none of it: it is made of the plain data (`copyOf`), so its
 * transcript is never walked.
 */
function stored(value: unknown): unknown {
  const raw: unknown = toRaw(value)
  if (Array.isArray(raw)) {
    let copy: unknown[] | null = null
    raw.forEach((item: unknown, index) => {
      const plain = stored(item)
      if (plain !== item) {
        copy ??= [...raw]
        copy[index] = plain
      }
    })
    return copy ?? raw
  }
  if (raw !== null && typeof raw === 'object') {
    let copy: Record<string, unknown> | null = null
    for (const [key, item] of Object.entries(raw)) {
      const plain = stored(item)
      if (plain !== item) {
        copy ??= { ...raw }
        copy[key] = plain
      }
    }
    return copy ?? raw
  }
  return raw
}

/** Whether `value` is a record this build wrote, read before its shape, which is that build's. */
function ofThisBuild(value: unknown): boolean {
  const record = z.object({ build: z.string() }).safeParse(value)
  return record.success && record.data.build === cacheBuild()
}

/**
 * The account's copy this build wrote; none when there is none, or the
 * cache is another build's or does not read, which is then dropped whole.
 */
export async function readAccountCopy(): Promise<AccountCopy | null> {
  const value = await database.transaction(ACCOUNT, 'readonly', (store) => store.get(ACCOUNT_KEY))
  if (value === undefined) {
    return null
  }
  // Another build's copy is dropped before its shape is read, which may
  // be that build's own: only a copy of this build that does not read is a
  // defect.
  const record = ofThisBuild(value) ? accountRecordSchema.safeParse(value) : null
  if (record === null || !record.success) {
    if (record !== null) {
      reportError(t('failures.readOffline'), schemaRefusal(record.error))
    }
    await deleteAccountCache()
    return null
  }
  return { user: record.data.user, state: record.data.state }
}

/**
 * The session that ended, whose copies are never written again: its account
 * and the user object the page held for it. A new sign-in, which brings a
 * user object of its own, writes again.
 */
let ended: { userId: string; user: object } | null = null

/** Whether the page may write copies for `userId`, the account of the session it holds. */
function writable(userId: string): boolean {
  return ended?.userId !== userId
}

/** Reads `value` as `schema` says a record of this build for `userId` is; another build's, another account's or one that does not read is none. */
function ours<S extends z.ZodType<{ userId: string }>>(schema: S, value: unknown, userId: string): z.infer<S> | null {
  if (value === undefined || !ofThisBuild(value)) {
    return null
  }
  const record = schema.safeParse(value)
  if (!record.success) {
    reportError(t('failures.readOffline'), schemaRefusal(record.error))
    return null
  }
  return record.data.userId === userId ? record.data : null
}

/**
 * The copy of conversation `id`'s metadata and agents this build wrote for
 * `userId`; none for none, or one that does not read. A database that
 * fails says so on the console, and the conversation has no copy then.
 */
export async function readConversationCopy(userId: string, id: string): Promise<ConversationCopy | null> {
  try {
    const value = await database.transaction(CONVERSATIONS, 'readonly', (store) => store.get(id))
    const record = ours(conversationRecordSchema, value, userId)
    if (!record) {
      return null
    }
    const { build: _build, userId: _user, ...copy } = record
    return copy
  } catch (error) {
    reportError(t('failures.readOffline'), error)
    return null
  }
}

/**
 * Replaces conversation `id`'s copy with `copy`, plain data behind no
 * proxy, unless the session of `userId` ended. A write that fails says so
 * on the console: the page works without the copy.
 */
export async function writeConversationCopy(userId: string, id: string, copy: ConversationCopy): Promise<void> {
  if (!writable(userId)) {
    return
  }
  const record = { build: cacheBuild(), userId, ...copy }
  try {
    await withRoom(id, () => database.transaction(CONVERSATIONS, 'readwrite', (store) => store.put(record, id)))
  } catch (error) {
    reportError(t('failures.keepOffline'), error)
  }
}

/**
 * What the disk holds of node `node` for `userId`: the last `DISK_WINDOW`
 * blocks of its latest run, as a view shows them at once, marked viewed
 * now; none when it holds none. A database that fails says so on the
 * console, and the node has no copy then.
 */
export async function readNodeCopy(userId: string, node: string): Promise<NodeCopy | null> {
  try {
    const record = await viewedNode(userId, node)
    const latest = record?.runs.at(-1)
    if (!record || !latest || latest.end < record.length) {
      return null
    }
    const start = Math.max(latest.start, latest.end - DISK_WINDOW)
    const window = await readRun(node, start, latest.end)
    if (!window) {
      return null
    }
    return {
      transcript: heldTranscript(record.length, window.blocks.length ? [{ start, blocks: window.blocks }] : [], record.version),
      failures: window.failures,
      instructions: record.instructions,
      summaries: record.summaries,
    }
  } catch (error) {
    reportError(t('failures.readOffline'), error)
    return null
  }
}

/**
 * The blocks of node `node` the disk holds whole before the index `end`, as
 * a view reads the page before its window there: up to `DISK_WINDOW` of
 * them, back to the start of the run that holds them, when that run holds
 * `edge` at `end` too, which the window starts with; none otherwise.
 */
export async function readNodeBlocks(
  userId: string,
  node: string,
  end: number,
  edge: string,
): Promise<{ window: TranscriptWindow, failures: Record<string, ProviderFailureFacts> } | null> {
  try {
    const value = await database.transaction(NODES, 'readonly', (store) => store.get(node))
    const run = ours(nodeRecordSchema, value, userId)?.runs.find((each) => each.start < end && each.end > end)
    if (!run) {
      return null
    }
    const start = Math.max(run.start, end - DISK_WINDOW)
    const read = await readRun(node, start, end + 1)
    if (read?.blocks.at(-1)?.id !== edge) {
      return null
    }
    return { window: { start, blocks: read.blocks.slice(0, -1) }, failures: read.failures }
  } catch (error) {
    reportError(t('failures.readOffline'), error)
    return null
  }
}

/** The node's record, marked viewed now; none for none. */
async function viewedNode(userId: string, node: string): Promise<NodeRecord | null> {
  return database.across([NODES], 'readwrite', (transaction) => {
    const nodes = transaction.objectStore(NODES)
    let record: NodeRecord | null = null
    const request = nodes.get(node)
    request.onsuccess = () => {
      record = ours(nodeRecordSchema, request.result, userId)
      if (record) {
        record = { ...record, viewedAt: Date.now() }
        nodes.put(record, node)
      }
    }
    return () => record
  })
}

/** The blocks `[start, end)` of node `node`; none when one of them is missing. */
async function readRun(node: string, start: number, end: number): Promise<{ blocks: HeldBlock[], failures: Record<string, ProviderFailureFacts> } | null> {
  const values = await database.transaction(BLOCKS, 'readonly', (store) =>
    store.getAll(IDBKeyRange.bound([node, start], [node, end], false, true)))
  if (values.length !== end - start) {
    return null
  }
  const blocks: HeldBlock[] = []
  const failures: Record<string, ProviderFailureFacts> = {}
  for (const value of values) {
    const record = blockRecordSchema.safeParse(value)
    if (!record.success) {
      reportError(t('failures.readOffline'), schemaRefusal(record.error))
      return null
    }
    blocks.push(record.data.block)
    if (record.data.failure) {
      failures[record.data.block.id] = record.data.failure
    }
  }
  return { blocks, failures }
}

/**
 * Writes what changed of node `node` for `userId`, its record and its
 * blocks in one transaction, unless the session of `userId` ended; each
 * block is plain data behind no proxy. A write that fails says so on the
 * console: the page works without it.
 */
export async function writeNode(userId: string, node: string, write: NodeWrite): Promise<void> {
  if (!writable(userId)) {
    return
  }
  try {
    await withRoom(write.conversation, () => database.across([NODES, BLOCKS], 'readwrite', (transaction) => {
      const nodes = transaction.objectStore(NODES)
      const blocks = transaction.objectStore(BLOCKS)
      // What its blocks take changes by what the write adds less what it
      // cuts or replaces, each about what its text takes, which is most of
      // it. A transaction runs its requests in order, so each read before a
      // change answers what the change replaces.
      const size = (value: unknown) => JSON.stringify(value).length
      let change = 0
      const replaced: IDBRequest[] = []
      if (write.cutFrom !== undefined) {
        const range = IDBKeyRange.bound([node, write.cutFrom], [node, Number.MAX_SAFE_INTEGER])
        replaced.push(blocks.getAll(range))
        blocks.delete(range)
      }
      for (const { index, block, failure } of write.blocks) {
        const record = failure ? { block, failure } : { block }
        change += size(record)
        replaced.push(blocks.get([node, index]))
        blocks.put(record, [node, index])
      }
      const held = nodes.get(node)
      held.onsuccess = () => {
        for (const read of replaced) {
          const values: unknown[] = Array.isArray(read.result) ? read.result : read.result === undefined ? [] : [read.result]
          for (const value of values) {
            change -= size(value)
          }
        }
        const before = ours(nodeRecordSchema, held.result, userId)
        nodes.put({
          build: cacheBuild(),
          userId,
          conversation: write.conversation,
          length: write.length,
          ...(write.version ? { version: write.version } : {}),
          runs: joinedRuns(write.replaceRuns ? [] : before?.runs ?? [], write),
          instructions: write.instructions,
          summaries: write.summaries,
          viewedAt: before?.viewedAt ?? Date.now(),
          bytes: (before?.bytes ?? 0) + change,
        } satisfies NodeRecord, node)
      }
      return () => {}
    }))
    scheduleEviction()
  } catch (error) {
    reportError(t('failures.keepOffline'), error)
  }
}

/**
 * The runs the disk holds whole after `write`: its runs among `before`,
 * which a cut and the transcript's new length end.
 */
function joinedRuns(before: readonly Run[], write: NodeWrite): Run[] {
  const end = Math.min(write.length, write.cutFrom ?? write.length)
  const runs = [...before.map((run) => ({ start: run.start, end: Math.min(run.end, end) })), ...write.runs]
    .filter((run) => run.end > run.start)
    .sort((a, b) => a.start - b.start)
  const joined: Run[] = []
  for (const run of runs) {
    const last = joined.at(-1)
    if (last && run.start <= last.end) {
      last.end = Math.max(last.end, run.end)
    } else {
      joined.push({ ...run })
    }
  }
  return joined
}

/** Deletes the account's cache, as signing out, a 401 or `session_ended` does. */
export async function deleteAccountCache(): Promise<void> {
  // All go at once, so whatever reads the cache from now on finds it gone.
  await Promise.all([ACCOUNT, CONVERSATIONS, NODES, BLOCKS].map((store) =>
    database.transaction(store, 'readwrite', (records) => records.clear()),
  ))
}

/**
 * Runs `write`. When the web browser refuses it for lack of room, the node
 * viewed longest ago goes, whole, until it fits; a node of `keep`, the
 * conversation being written, goes only when no other is left.
 */
async function withRoom<T>(keep: string | null, write: () => Promise<T>): Promise<T> {
  for (;;) {
    try {
      return await write()
    } catch (error) {
      if (!(error instanceof DOMException) || error.name !== 'QuotaExceededError') {
        throw error
      }
      const oldest = await viewedLongestAgo(keep)
      if (oldest === null) {
        throw error
      }
      while (!(await evictStep(oldest))) {
        // Each step deletes a batch; the node's record goes with its last.
      }
    }
  }
}

/** The node viewed longest ago, one not of conversation `keep` first; none when the disk holds none. */
async function viewedLongestAgo(keep: string | null): Promise<string | null> {
  return database.across([NODES], 'readonly', (transaction) => {
    let found: string | null = null
    let other: string | null = null
    const cursor = transaction.objectStore(NODES).index('viewedAt').openCursor()
    cursor.onsuccess = () => {
      const at = cursor.result
      if (!at) {
        return
      }
      const conversation = (at.value as { conversation?: unknown }).conversation
      found ??= String(at.primaryKey)
      if (conversation !== keep) {
        other = String(at.primaryKey)
        return
      }
      at.continue()
    }
    return () => other ?? found
  })
}

/**
 * Deletes a batch of node `node`'s blocks, and its record with its last,
 * with the conversation's record once its root goes; answers whether the
 * node is gone.
 */
async function evictStep(node: string): Promise<boolean> {
  return database.across([NODES, BLOCKS, CONVERSATIONS], 'readwrite', (transaction) => {
    const blocks = transaction.objectStore(BLOCKS)
    const range = IDBKeyRange.bound([node, 0], [node, Number.MAX_SAFE_INTEGER])
    let gone = false
    const keys = blocks.getAllKeys(range, EVICTION_BATCH)
    keys.onsuccess = () => {
      for (const key of keys.result) {
        blocks.delete(key)
      }
      if (keys.result.length < EVICTION_BATCH) {
        gone = true
        const nodes = transaction.objectStore(NODES)
        const record = nodes.get(node)
        record.onsuccess = () => {
          const conversation = (record.result as { conversation?: unknown } | undefined)?.conversation
          // The root's node is the conversation's id: the conversation's
          // record goes with it.
          if (conversation === node) {
            transaction.objectStore(CONVERSATIONS).delete(node)
          }
          nodes.delete(node)
        }
      }
    }
    return () => gone
  })
}

/** The bound the disk keeps under: `DISK_BYTES`, or what the browser grants if less. */
async function diskBound(): Promise<number> {
  const quota = (await navigator.storage?.estimate?.())?.quota
  return quota === undefined ? DISK_BYTES : Math.min(DISK_BYTES, quota)
}

/** How many bytes the nodes' blocks take, about, and the node viewed longest ago. */
async function diskUse(): Promise<{ bytes: number, oldest: string | null }> {
  return database.across([NODES], 'readonly', (transaction) => {
    let bytes = 0
    let oldest: string | null = null
    const cursor = transaction.objectStore(NODES).index('viewedAt').openCursor()
    cursor.onsuccess = () => {
      const at = cursor.result
      if (!at) {
        return
      }
      oldest ??= String(at.primaryKey)
      const counted = (at.value as { bytes?: unknown }).bytes
      bytes += typeof counted === 'number' ? counted : 0
      at.continue()
    }
    return () => ({ bytes, oldest })
  })
}

let evicting = false

/**
 * Keeps the disk under its bound once the page is idle: one batch of the
 * node viewed longest ago at each idle moment, until the disk fits.
 */
function scheduleEviction(): void {
  if (evicting) {
    return
  }
  evicting = true
  const step = () => {
    void (async () => {
      const [use, bound] = await Promise.all([diskUse(), diskBound()])
      if (use.bytes <= bound || use.oldest === null) {
        evicting = false
        return
      }
      await evictStep(use.oldest)
      idle(step)
    })().catch((error: unknown) => {
      evicting = false
      reportError(t('failures.keepOffline'), error)
    })
  }
  idle(step)
}

/** Runs `step` when the page is idle; a browser without idle callbacks runs it after the current task. */
function idle(step: () => void): void {
  if (typeof requestIdleCallback === 'function') {
    requestIdleCallback(step)
  } else {
    setTimeout(step, 0)
  }
}

/** What the account's cache reads of the page's session. */
export interface CacheSession {
  readonly user: UserDto | null
}

/** What the account's cache reads of the page's product state, and how it shows the copy. */
export interface CacheProduct {
  readonly read: ProductState | null
  showCopy(state: ProductState): void
}

/**
 * The account's cache from the page's start on: the copy is read once,
 * shown at once for the account the session names until the channel's
 * first snapshot, and kept as the state changes. `signInOffline` is the
 * account a first session check that cannot reach the backend signs in as,
 * whose copy then shows; `settled` resolves once the writes asked for so
 * far are done.
 */
export function startAccountCache(session: CacheSession, product: CacheProduct) {
  const copy = readAccountCopy().catch((error: unknown) => {
    // A database that fails leaves the page as it is without a copy.
    reportError(t('failures.readOffline'), error)
    return null
  })
  const stopShowing = watch(
    () => session.user?.id,
    (id) => {
      if (id) {
        void copy.then((kept) => {
          if (kept && kept.user.id === id && session.user?.id === id) {
            product.showCopy(kept.state)
          }
        })
      }
    },
    { immediate: true },
  )
  const keeping = keepAccountCopy(() =>
    session.user && product.read ? { user: session.user, state: product.read } : null,
  )
  // The cache is the account's: a 401 or the channel's `session_ended` takes
  // it along at once, whatever the page then learns of the session, and no
  // write of the session that ended brings it back.
  const stopExpiry = onSessionExpired((over) => {
    if (!over) {
      return
    }
    const user = session.user
    if (user) {
      ended = { userId: user.id, user: toRaw(user) }
    }
    void deleteAccountCache().catch((error: unknown) => reportError(t('failures.keepOffline'), error))
  })
  return {
    async signInOffline(): Promise<UserDto | null> {
      const kept = await copy
      if (!kept) {
        return null
      }
      product.showCopy(kept.state)
      return kept.user
    },
    settled: keeping.settled,
    stop(): void {
      stopShowing()
      stopExpiry()
      keeping.stop()
    },
  }
}

/**
 * Keeps the account's copy as `account` changes: each change is written,
 * the latest one when several came during a write; nothing while the page
 * is signed out. The first copy of an account replaces another account's
 * cache whole.
 */
function keepAccountCopy(account: () => AccountCopy | null): { settled(): Promise<void>; stop(): void } {
  let next: AccountCopy | null = null
  let writing: Promise<void> | null = null
  /** The account whose cache this page knows the database holds. */
  let owner: string | null = null

  async function drain(): Promise<void> {
    try {
      while (next) {
        const { user, state } = next
        next = null
        if (owner !== user.id) {
          await replaceOtherAccount(user.id)
          owner = user.id
        }
        // Signed out meanwhile, or the session ended: the cache is gone, and stays gone.
        if (!account() || !writable(user.id)) {
          return
        }
        const record = stored({ build: cacheBuild(), user, state })
        await withRoom(null, () => database.transaction(ACCOUNT, 'readwrite', (store) => store.put(record, ACCOUNT_KEY)))
      }
    } catch (error) {
      // The page works without the copy; it goes on showing the backend's state.
      reportError(t('failures.keepOffline'), error)
    }
  }

  // Heard at once, so a write asked for is one `settled` waits for.
  const stop = watch(account, (copy) => {
    // A new sign-in of the account whose session ended writes again.
    if (copy && ended?.userId === copy.user.id && ended.user !== toRaw(copy.user)) {
      ended = null
    }
    next = copy
    // Cleared once the writes end, after it was set.
    writing ??= drain().finally(() => {
      writing = null
    })
  }, { immediate: true, flush: 'sync' })
  return {
    settled: async () => {
      await writing
    },
    stop: () => {
      stop()
      next = null
    },
  }
}

/** Drops the cache when it is another account's than `userId`. */
async function replaceOtherAccount(userId: string): Promise<void> {
  const value = await database.transaction(ACCOUNT, 'readonly', (store) => store.get(ACCOUNT_KEY))
  const user = z.object({ user: z.object({ id: z.string() }) }).safeParse(value)
  if (value !== undefined && (!user.success || user.data.user.id !== userId)) {
    await deleteAccountCache()
  }
}
