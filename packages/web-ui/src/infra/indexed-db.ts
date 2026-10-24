import { environmental, errorFields } from './errors'
import { pageLog } from './log'

/**
 * The failures of the browser's storage itself, which are the environment's
 * (`environmental`): a full quota, and storage that is unavailable, as in a
 * private window or after the browser lost its files. Any other failure of
 * an operation, such as a ConstraintError, is this page's defect.
 */
const STORAGE_FAILURES = new Set(['QuotaExceededError', 'UnknownError', 'InvalidStateError'])

/** `error`, marked as the environment's when the browser's storage itself failed. */
function storageFailure<E>(error: E): E {
  return error instanceof DOMException && STORAGE_FAILURES.has(error.name) ? environmental(error) : error
}

/** Whether `error` is the browser's NotFoundError: a store or an index that is not there. */
function isNotFound(error: unknown): boolean {
  return error instanceof DOMException && error.name === 'NotFoundError'
}

/**
 * One IndexedDB database of the page: opened on first use and kept open, so
 * that a write issued as the page hides starts at once, and opened again
 * whenever the web browser or another page closed the connection.
 *
 * The web browser closes a connection on its own when the site's data is
 * cleared or evicted, or a frozen page is restored, and says so with a
 * `close` event; another page that upgrades or deletes the database asks with
 * `versionchange`, and this page lets it go. Safari also closes an idle
 * connection in a background tab without an event, which the next
 * transaction on it finds out with an InvalidStateError.
 *
 * The data is the web browser's and disposable, so the page never waits on
 * what it cannot use: a database it cannot open at its version, as one a
 * newer build upgraded (VersionError) or whose upgrade failed, and one
 * missing a store or an index it needs, is deleted and created anew, once
 * for an operation, and the operation goes on; each deletion is logged. A
 * page of another build in another tab lets the deletion through at its
 * `versionchange`, and opens the database again at its own version on its
 * next transaction; each page drops what the other wrote, and neither stops.
 * An open or a deletion that waits for a page that does not let go, as a
 * frozen one, keeps the operations behind it waiting: a caller whose work
 * must not wait for the database, such as the page's start, does not await
 * it. A record the page cannot read is dropped by the code that decodes it.
 */
export class IndexedDatabase {
  #database: IDBDatabase | null = null
  #opening: Promise<IDBDatabase> | null = null
  #recreating: Promise<IDBDatabase> | null = null
  readonly #deleted = new Set<() => void>()

  /**
   * `upgrade` turns the database of `oldVersion` into `version`, inside the
   * open's versionchange transaction. `indexes` names the indexes each store
   * has once it did, which the page's operations use; one missing from a
   * database of this version makes it unusable.
   */
  constructor(
    private readonly name: string,
    private readonly version: number,
    private readonly upgrade: (request: IDBOpenDBRequest, oldVersion: number) => void,
    private readonly indexes: Readonly<Record<string, readonly string[]>> = {},
  ) {}

  /**
   * Calls `listener` whenever the database was deleted, by this page or by
   * another, so that its owner writes again what it holds in memory as saved.
   * Answers the call that stops it.
   */
  onDeleted(listener: () => void): () => void {
    this.#deleted.add(listener)
    return () => void this.#deleted.delete(listener)
  }

  /**
   * Runs `operation` in a transaction on `store` and resolves with its
   * request's result once the transaction commits. On an open connection
   * the transaction starts before this returns.
   */
  transaction<T>(
    store: string,
    mode: IDBTransactionMode,
    operation: (store: IDBObjectStore) => IDBRequest<T>,
  ): Promise<T> {
    return this.across([store], mode, (transaction) => {
      const request = operation(transaction.objectStore(store))
      return () => request.result
    })
  }

  /**
   * Runs `operation` in one transaction over `stores`, all of whose changes
   * commit or none, and resolves with what the reader it returns gives once
   * the transaction commits. On an open connection the transaction starts
   * before this returns.
   */
  across<T>(
    stores: readonly string[],
    mode: IDBTransactionMode,
    operation: (transaction: IDBTransaction) => () => T,
  ): Promise<T> {
    return this.#database
      ? this.#run(this.#database, stores, mode, operation, false)
      : this.#open().then((database) => this.#run(database, stores, mode, operation, false))
  }

  /**
   * Closes the connection: transactions already started finish, and those
   * waiting for the connection to open start on it before it closes. The
   * next transaction opens a new one.
   */
  close(): void {
    this.#database?.close()
    this.#database = null
    const opening = this.#opening
    this.#opening = null
    // The operations waiting for this open were chained to it before this
    // reaction, so their transactions start first. A failed open has
    // nothing to close, and its error reaches those operations.
    opening?.then(
      (database) => database.close(),
      () => {},
    )
  }

  #run<T>(
    database: IDBDatabase,
    stores: readonly string[],
    mode: IDBTransactionMode,
    operation: (transaction: IDBTransaction) => () => T,
    retried: boolean,
  ): Promise<T> {
    let transaction: IDBTransaction | null = null
    let result: () => T
    let unusable = false
    try {
      transaction = database.transaction([...stores], mode)
      try {
        result = operation(transaction)
      } catch (error) {
        unusable = isNotFound(error) && this.#lacks(database, stores, transaction)
        transaction.abort()
        throw error
      }
    } catch (error) {
      unusable ||= transaction === null && isNotFound(error) && this.#lacks(database, stores, null)
      if (unusable && !retried) {
        return this.#recreate(database, error).then((created) => this.#run(created, stores, mode, operation, true))
      }
      if (retried || !(error instanceof DOMException) || error.name !== 'InvalidStateError') {
        return Promise.reject(storageFailure(error))
      }
      // The connection closed without a close event. No transaction was
      // made, so nothing of `operation` ran: it runs once on a new one.
      return this.#again(database, stores, mode, operation)
    }
    return new Promise<T>((resolve, reject) => {
      transaction.oncomplete = () => resolve(result())
      // A failed request aborts its transaction, so `abort` reports every
      // failure; its `error` event comes first and is left to it.
      transaction.onabort = () => {
        const error = transaction.error ?? new DOMException(`A transaction on ${this.name} was aborted.`, 'AbortError')
        if (retried || error.name !== 'AbortError') {
          reject(storageFailure(error))
          return
        }
        // The web browser closed the connection under the transaction, which
        // committed nothing: it runs once on a new one.
        resolve(this.#again(database, stores, mode, operation))
      }
    })
  }

  /** Whether `database` lacks a store of `stores`, or an index `indexes` names for one, as `transaction` shows. */
  #lacks(database: IDBDatabase, stores: readonly string[], transaction: IDBTransaction | null): boolean {
    return stores.some(
      (store) =>
        !database.objectStoreNames.contains(store) ||
        (transaction !== null &&
          (this.indexes[store] ?? []).some((index) => !transaction.objectStore(store).indexNames.contains(index))),
    )
  }

  #again<T>(
    closed: IDBDatabase,
    stores: readonly string[],
    mode: IDBTransactionMode,
    operation: (transaction: IDBTransaction) => () => T,
  ): Promise<T> {
    this.#forget(closed)
    return this.#open().then((database) => this.#run(database, stores, mode, operation, true))
  }

  #open(): Promise<IDBDatabase> {
    if (this.#opening) {
      return this.#opening
    }
    const opening: Promise<IDBDatabase> = this.#connect().then(
      (database) => {
        if (this.#opening === opening) {
          this.#opening = null
          this.#keep(database)
        }
        return database
      },
      (error) => {
        if (this.#opening === opening) {
          this.#opening = null
        }
        throw error
      },
    )
    this.#opening = opening
    return opening
  }

  /** Opens the database, deleting it first once when this page cannot open it as it is. */
  async #connect(): Promise<IDBDatabase> {
    try {
      return await this.#request()
    } catch (error) {
      if (!(error instanceof DOMException) || STORAGE_FAILURES.has(error.name)) {
        throw storageFailure(error)
      }
      // A VersionError, or an upgrade that failed: what the database holds is
      // not what this build reads.
      pageLog.warn('storage.dropped', { database: this.name, ...errorFields(error) })
    }
    await this.#delete()
    // Failing again is not the browser's storage failing, unless it says so.
    return this.#request().catch((error) => {
      throw storageFailure(error)
    })
  }

  /** One `open`, answering the connection or why there is none. */
  #request(): Promise<IDBDatabase> {
    return new Promise<IDBDatabase>((resolve, reject) => {
      let blocked = false
      // Whatever aborts the upgrade, the open request's error is an
      // AbortError; the transaction's own says whether the upgrade threw or
      // the storage failed under it.
      let aborted: DOMException | null = null
      const request = indexedDB.open(this.name, this.version)
      request.onupgradeneeded = (event) => {
        const upgrading = request.transaction!
        upgrading.addEventListener('abort', () => (aborted = upgrading.error))
        this.upgrade(request, event.oldVersion)
      }
      request.onsuccess = () => {
        const database = request.result
        if (blocked) {
          // Its operations were already refused; the next one opens again.
          database.close()
          return
        }
        resolve(database)
      }
      request.onerror = () => reject(aborted ?? request.error)
      request.onblocked = () => {
        blocked = true
        reject(environmental(new Error(`Another page keeps an older version of ${this.name} open.`)))
      }
    })
  }

  /**
   * Deletes the database, as the page's own connection and every other
   * page's let go of it at `versionchange`. A page that keeps a transaction
   * running first blocks the deletion for a moment, and one that does not
   * let go at all keeps it waiting: it finishes once that page closes.
   */
  #delete(): Promise<void> {
    this.#database?.close()
    this.#database = null
    return new Promise<void>((resolve, reject) => {
      const request = indexedDB.deleteDatabase(this.name)
      request.onsuccess = () => {
        this.#deleted.forEach((listener) => listener())
        resolve()
      }
      request.onerror = () => reject(environmental(request.error))
    })
  }

  /**
   * A new database in place of `broken`, which a transaction found unusable;
   * the operations that find the same one at once share the deletion, and one
   * that finds it already replaced uses the new one.
   */
  #recreate(broken: IDBDatabase, error: unknown): Promise<IDBDatabase> {
    if (this.#recreating) {
      return this.#recreating
    }
    if (this.#database !== broken) {
      return this.#database ? Promise.resolve(this.#database) : this.#open()
    }
    pageLog.warn('storage.dropped', { database: this.name, ...errorFields(error) })
    const recreating = this.#delete()
      .then(() => this.#open())
      .finally(() => {
        if (this.#recreating === recreating) {
          this.#recreating = null
        }
      })
    this.#recreating = recreating
    return recreating
  }

  #keep(database: IDBDatabase): void {
    this.#database = database
    database.addEventListener('close', () => this.#forget(database))
    database.addEventListener('versionchange', (event) => {
      this.#forget(database)
      database.close()
      if (event.newVersion === null) {
        // Another page deletes the database: what this page saved is gone.
        this.#deleted.forEach((listener) => listener())
      }
    })
  }

  #forget(database: IDBDatabase): void {
    if (this.#database === database) {
      this.#database = null
    }
  }
}
