import { shallowRef } from 'vue'
import { environmental } from '@demicodes/web-ui/infra/errors'

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
 */
/**
 * Set once a newer build has upgraded a database this page uses, as a page
 * of that build in another tab does: this page would write in a shape the
 * newer build has left behind, so it loads itself again
 * (`web-application.md` § A page of another build). The value names the
 * database and the version this page knows, which the page loads itself
 * again for only once.
 */
export const newerStorage = shallowRef<string | null>(null)

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

export class IndexedDatabase {
  #database: IDBDatabase | null = null
  #opening: Promise<IDBDatabase> | null = null

  /** `upgrade` turns the database of `oldVersion` into `version`, inside the open's versionchange transaction. */
  constructor(
    private readonly name: string,
    private readonly version: number,
    private readonly upgrade: (request: IDBOpenDBRequest, oldVersion: number) => void,
  ) {}

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
    let transaction: IDBTransaction
    try {
      transaction = database.transaction([...stores], mode)
    } catch (error) {
      if (retried || !(error instanceof DOMException) || error.name !== 'InvalidStateError') {
        return Promise.reject(storageFailure(error))
      }
      // The connection closed without a close event. No transaction was
      // made, so nothing of `operation` ran: it runs once on a new one.
      return this.#again(database, stores, mode, operation)
    }
    return new Promise<T>((resolve, reject) => {
      const result = operation(transaction)
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
    const opening = new Promise<IDBDatabase>((resolve, reject) => {
      let blocked = false
      const request = indexedDB.open(this.name, this.version)
      request.onupgradeneeded = (event) => this.upgrade(request, event.oldVersion)
      request.onsuccess = () => {
        const database = request.result
        if (blocked) {
          // Its operations were already refused; the next one opens again.
          database.close()
          return
        }
        if (this.#opening === opening) {
          this.#opening = null
          this.#keep(database)
        }
        resolve(database)
      }
      request.onerror = () => {
        if (this.#opening === opening) {
          this.#opening = null
        }
        if (request.error?.name === 'VersionError') {
          newerStorage.value = `${this.name} ${this.version}`
        }
        reject(environmental(request.error))
      }
      request.onblocked = () => {
        blocked = true
        if (this.#opening === opening) {
          this.#opening = null
        }
        reject(environmental(new Error(`Another page keeps an older version of ${this.name} open.`)))
      }
    })
    this.#opening = opening
    return opening
  }

  #keep(database: IDBDatabase): void {
    this.#database = database
    database.addEventListener('close', () => this.#forget(database))
    database.addEventListener('versionchange', () => {
      this.#forget(database)
      database.close()
    })
  }

  #forget(database: IDBDatabase): void {
    if (this.#database === database) {
      this.#database = null
    }
  }
}
