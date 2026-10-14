import { shallowRef } from 'vue'

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
    return this.#database
      ? this.#run(this.#database, store, mode, operation, false)
      : this.#open().then((database) => this.#run(database, store, mode, operation, false))
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
    store: string,
    mode: IDBTransactionMode,
    operation: (store: IDBObjectStore) => IDBRequest<T>,
    retried: boolean,
  ): Promise<T> {
    let transaction: IDBTransaction
    try {
      transaction = database.transaction(store, mode)
    } catch (error) {
      if (retried || !(error instanceof DOMException) || error.name !== 'InvalidStateError') {
        return Promise.reject(error)
      }
      // The connection closed without a close event. No transaction was
      // made, so nothing of `operation` ran: it runs once on a new one.
      return this.#again(database, store, mode, operation)
    }
    return new Promise<T>((resolve, reject) => {
      const request = operation(transaction.objectStore(store))
      transaction.oncomplete = () => resolve(request.result)
      // A failed request aborts its transaction, so `abort` reports every
      // failure; its `error` event comes first and is left to it.
      transaction.onabort = () => {
        const error = transaction.error ?? new DOMException(`A transaction on ${this.name} was aborted.`, 'AbortError')
        if (retried || error.name !== 'AbortError') {
          reject(error)
          return
        }
        // The web browser closed the connection under the transaction, which
        // committed nothing: it runs once on a new one.
        resolve(this.#again(database, store, mode, operation))
      }
    })
  }

  #again<T>(
    closed: IDBDatabase,
    store: string,
    mode: IDBTransactionMode,
    operation: (store: IDBObjectStore) => IDBRequest<T>,
  ): Promise<T> {
    this.#forget(closed)
    return this.#open().then((database) => this.#run(database, store, mode, operation, true))
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
        reject(request.error)
      }
      request.onblocked = () => {
        blocked = true
        if (this.#opening === opening) {
          this.#opening = null
        }
        reject(new Error(`Another page keeps an older version of ${this.name} open.`))
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
