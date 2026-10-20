import { toRaw, watch } from 'vue'
import { z } from 'zod'
import {
  blockSchema,
  instructionEntrySchema,
  providerFailureFactsSchema,
  schemaRefusal,
  subagentJobSchema,
  versionsSchema,
} from '@demicodes/protocol'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { onSessionExpired } from '../api/client'
import { productStateSchema, userDtoSchema, type ProductState, type UserDto } from '../api/generated/web-api'
import { IndexedDatabase } from '@demicodes/web-ui/infra/indexed-db'
import { t } from './locales/text'

/**
 * The account's cache for opening offline: the page's copy of the product state
 * with the account the session names, and, for each conversation the user
 * opened on this device, the latest window of its root agent as the page held
 * it, with its agents. It is read only while the backend cannot be reached, and
 * it is disposable: written by one build and read by that build alone, a record
 * of another build is dropped, never migrated, and it is the account's alone.
 */

/** The build the records belong to; Vite's development server names none. */
function cacheBuild(): string {
  return import.meta.env.DEMI_WEB_BUILD ?? 'development'
}

const ACCOUNT = 'account'
const CONVERSATIONS = 'conversations'
/**
 * When the user last opened each kept conversation, `{ openedAt }` by its
 * id: a record of its own, so opening a conversation writes these few bytes
 * and never its copy, and the copies opened longest ago are found by its
 * index when room runs out. A copy and its record are written and deleted
 * together, so every kept copy has one.
 */
const OPENED = 'opened'
/** The one key of the account's record. */
const ACCOUNT_KEY = 'account'

/**
 * The database's layout. Its records are disposable, so a change of their
 * shape needs no new version: a record of another build is dropped. A new
 * layout drops every store of the one before, records and all, as a cache
 * of another build is dropped.
 */
const database = new IndexedDatabase('demi-offline', 2, (request) => {
  const db = request.result
  for (const name of [...db.objectStoreNames]) {
    db.deleteObjectStore(name)
  }
  db.createObjectStore(ACCOUNT)
  db.createObjectStore(CONVERSATIONS)
  db.createObjectStore(OPENED).createIndex('openedAt', 'openedAt')
})

const accountRecordSchema = z.object({
  build: z.string(),
  user: userDtoSchema,
  state: productStateSchema,
})

/** A child of the conversation as its job says it, without what only the job's frames carry. */
const agentCopySchema = subagentJobSchema.omit({ parentSessionId: true, result: true })

/** A block as the page held it: whole, or light when it was read from a page. */
const heldBlockSchema = z.intersection(
  blockSchema,
  z.object({ light: z.literal(true).optional(), versions: versionsSchema.optional() }),
)

const conversationRecordSchema = z.object({
  build: z.string(),
  userId: z.string(),
  /** The root agent's latest window, and how many blocks its transcript held. */
  transcript: z.object({
    length: z.int().nonnegative(),
    start: z.int().nonnegative(),
    blocks: z.array(heldBlockSchema),
  }),
  failures: z.record(z.string(), providerFailureFactsSchema),
  instructions: z.array(instructionEntrySchema),
  summaries: z.record(z.string(), z.int().nonnegative()),
  agents: z.array(agentCopySchema),
})

export interface AccountCopy {
  user: UserDto
  state: ProductState
}

export type ConversationCopy = Omit<z.infer<typeof conversationRecordSchema>, 'build' | 'userId'>
export type AgentCopy = z.infer<typeof agentCopySchema>

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
 * The copy of conversation `id` this build wrote for `userId`; a record of
 * another build, or one that does not read, is dropped. A database that
 * fails says so on the console, and the conversation has no copy then.
 */
export async function readConversationCopy(userId: string, id: string): Promise<ConversationCopy | null> {
  try {
    return await readConversation(userId, id)
  } catch (error) {
    reportError(t('failures.readOffline'), error)
    return null
  }
}

async function readConversation(userId: string, id: string): Promise<ConversationCopy | null> {
  const value = await database.transaction(CONVERSATIONS, 'readonly', (store) => store.get(id))
  if (value === undefined) {
    return null
  }
  const record = ofThisBuild(value) ? conversationRecordSchema.safeParse(value) : null
  if (record === null || !record.success || record.data.userId !== userId) {
    if (record !== null && !record.success) {
      reportError(t('failures.readOffline'), schemaRefusal(record.error))
    }
    await forget(id)
    return null
  }
  const { build: _build, userId: _user, ...copy } = record.data
  return copy
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

/** What hears that a conversation's copy left the cache, by its id. */
const dropListeners = new Set<(id: string) => void>()

/**
 * Hears of each conversation whose copy leaves the cache while the account
 * keeps it: the copy opened longest ago, which goes to make room, or one
 * that did not read. Returns what stops it.
 */
export function onConversationCopyDropped(listener: (id: string) => void): () => void {
  dropListeners.add(listener)
  return () => {
    dropListeners.delete(listener)
  }
}

/**
 * Replaces the copy of conversation `id` whole with `copy`, and when the
 * user last opened it with `openedAt`, as `Date.now()`, both in one
 * transaction, so every kept copy has the recency room is made by; unless
 * the session of `userId` ended. `copy` is plain data, behind no proxy,
 * which the database copies as it is. A write that fails says so on the
 * console: the page works without the copy.
 */
export async function writeConversationCopy(userId: string, id: string, copy: ConversationCopy, openedAt: number): Promise<void> {
  if (!writable(userId)) {
    return
  }
  const record = { build: cacheBuild(), userId, ...copy }
  try {
    await withRoom(id, () => database.across([CONVERSATIONS, OPENED], 'readwrite', (transaction) => {
      transaction.objectStore(CONVERSATIONS).put(record, id)
      transaction.objectStore(OPENED).put({ openedAt }, id)
      return () => {}
    }))
  } catch (error) {
    reportError(t('failures.keepOffline'), error)
  }
}

/**
 * Records that the user opened conversation `id` at `openedAt`, as
 * `Date.now()`, where the cache keeps its copy, which stays as it is; a
 * conversation without a copy gets its recency with its copy. Nothing
 * once the session of `userId` ended. A write that fails says so on the
 * console.
 */
export async function writeConversationOpened(userId: string, id: string, openedAt: number): Promise<void> {
  if (!writable(userId)) {
    return
  }
  try {
    await withRoom(id, () => database.across([CONVERSATIONS, OPENED], 'readwrite', (transaction) => {
      const kept = transaction.objectStore(CONVERSATIONS).getKey(id)
      kept.onsuccess = () => {
        if (kept.result !== undefined) {
          transaction.objectStore(OPENED).put({ openedAt }, id)
        }
      }
      return () => {}
    }))
  } catch (error) {
    reportError(t('failures.keepOffline'), error)
  }
}

/** Deletes conversation `id`'s copy and when it was opened, together, and says the copy is gone. */
async function forget(id: string): Promise<void> {
  await database.across([CONVERSATIONS, OPENED], 'readwrite', (transaction) => {
    transaction.objectStore(CONVERSATIONS).delete(id)
    transaction.objectStore(OPENED).delete(id)
    return () => {}
  })
  for (const listener of dropListeners) {
    listener(id)
  }
}

/** Deletes the account's cache, as signing out, a 401 or `session_ended` does. */
export async function deleteAccountCache(): Promise<void> {
  // All go at once, so whatever reads the cache from now on finds it gone.
  await Promise.all([ACCOUNT, CONVERSATIONS, OPENED].map((store) =>
    database.transaction(store, 'readwrite', (records) => records.clear()),
  ))
}

/**
 * Runs `write`. When the web browser refuses it for lack of room, the
 * copies of the conversations opened longest ago go, one at a time, until
 * it fits; the copy of `keep`, the one being written, stays.
 */
async function withRoom<T>(keep: string | null, write: () => Promise<T>): Promise<T> {
  for (;;) {
    try {
      return await write()
    } catch (error) {
      if (!(error instanceof DOMException) || error.name !== 'QuotaExceededError') {
        throw error
      }
      const oldest = await openedLongestAgo(keep)
      if (oldest === null) {
        throw error
      }
      await forget(oldest)
    }
  }
}

/** The conversation, other than `keep`, whose copy was opened longest ago; none when no other is kept. */
async function openedLongestAgo(keep: string | null): Promise<string | null> {
  const keys = await database.transaction(OPENED, 'readonly', (records) =>
    records.index('openedAt').getAllKeys(null, 2),
  )
  return keys.find((key): key is string => typeof key === 'string' && key !== keep) ?? null
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
