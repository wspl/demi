import { abortable, errorMessage } from '@demicodes/utils'
import type { ControlService } from '../storage/control'
import {
  catalogSnapshotSchema,
  type CatalogSnapshot,
  type ModelCatalogRecord,
} from '../storage/model-catalog'

export const MODEL_CATALOG_TTL_MS = 15 * 60 * 1000
const RETRY_DELAY_MS = 60 * 1000
const REQUEST_TIMEOUT_MS = 10 * 1000

type CatalogStore = Pick<ControlService,
  'getModelCatalog' | 'putModelCatalog' | 'deleteModelCatalog'>
interface Entry {
  key: string
  record: Promise<ModelCatalogRecord | null>
  controller: AbortController
  refresh: Promise<CatalogSnapshot> | null
  failure: { message: string; retryAt: number } | null
}

/** One durable metadata snapshot and one refresh per provider entry. */
export class ModelCatalogCache {
  private readonly entries = new Map<string, Entry>()
  private closed = false

  constructor(
    private readonly store: CatalogStore,
    private readonly now: () => number = Date.now,
    private readonly timeoutMs = REQUEST_TIMEOUT_MS,
  ) {}

  async get(
    providerId: string,
    key: string,
    fetch: (signal: AbortSignal) => Promise<CatalogSnapshot>,
    force = false,
  ): Promise<CatalogSnapshot> {
    if (this.closed)
      throw new Error('Model catalog cache is closed')
    let entry = this.entries.get(providerId)
    if (!entry || entry.key !== key) {
      entry?.controller.abort()
      entry = {
        key,
        record: this.store.getModelCatalog(providerId).then(record => {
          if (record?.catalog.models.some(model => model.providerId !== providerId))
            throw new Error('Stored model catalog contains another provider identity')
          return record?.key === key ? record : null
        }),
        controller: new AbortController(),
        refresh: null,
        failure: null,
      }
      this.entries.set(providerId, entry)
    }
    const record = await entry.record
    entry.controller.signal.throwIfAborted()
    if (!force && !entry.failure && record && this.now() - record.checkedAt < MODEL_CATALOG_TTL_MS)
      return structuredClone(record.catalog)
    if (!force && entry.failure && this.now() < entry.failure.retryAt) {
      if (record)
        return staleCatalog(record.catalog, entry.failure?.message ?? null)
      throw new Error(entry.failure?.message ?? 'Model catalog is unavailable')
    }
    if (!entry.refresh) {
      const pending = this.refresh(providerId, entry, fetch)
      entry.refresh = pending
      // Stale readers return immediately. The rejection is retained as entry.failure;
      // cold and explicit-refresh readers still observe it through pending.
      void pending.catch(() => {}).finally(() => {
        if (entry.refresh === pending)
          entry.refresh = null
      })
    }
    if (record && !force)
      return staleCatalog(record.catalog, entry.failure?.message ?? null)
    return entry.refresh
  }

  async invalidate(providerId: string): Promise<void> {
    const entry = this.entries.get(providerId)
    this.entries.delete(providerId)
    entry?.controller.abort()
    await this.store.deleteModelCatalog(providerId)
    await Promise.allSettled([entry?.refresh])
  }

  async close(): Promise<void> {
    this.closed = true
    const entries = [...this.entries.values()]
    this.entries.clear()
    for (const entry of entries)
      entry.controller.abort()
    await Promise.allSettled(entries.map(entry => entry.refresh))
  }

  private async refresh(
    providerId: string,
    entry: Entry,
    fetch: (signal: AbortSignal) => Promise<CatalogSnapshot>,
  ): Promise<CatalogSnapshot> {
    const timeout = new AbortController()
    const timer = setTimeout(() => timeout.abort(new Error('Model catalog request timed out')), this.timeoutMs)
    const signal = AbortSignal.any([entry.controller.signal, timeout.signal])
    try {
      const catalog = catalogSnapshotSchema.parse(await abortable(fetch(signal), signal))
      signal.throwIfAborted()
      if (catalog.models.some(model => model.providerId !== providerId))
        throw new Error('Model catalog contains another provider identity')
      if (catalog.stale)
        throw new Error(catalog.warnings.join('; ') || 'Provider returned a stale model catalog')
      const record = { key: entry.key, checkedAt: this.now(), catalog }
      await this.store.putModelCatalog(providerId, record)
      entry.controller.signal.throwIfAborted()
      entry.record = Promise.resolve(record)
      entry.failure = null
      return structuredClone(catalog)
    } catch (error) {
      entry.controller.signal.throwIfAborted()
      const failure = timeout.signal.aborted ? timeout.signal.reason : error
      entry.failure = { message: errorMessage(failure), retryAt: this.now() + RETRY_DELAY_MS }
      const record = await entry.record
      if (record)
        return staleCatalog(record.catalog, entry.failure.message)
      throw failure
    } finally {
      clearTimeout(timer)
    }
  }
}

function staleCatalog(catalog: CatalogSnapshot, warning: string | null): CatalogSnapshot {
  const copy = structuredClone(catalog)
  return {
    ...copy,
    stale: true,
    models: copy.models.map(model => ({ ...model, stale: true })),
    warnings: warning ? [...copy.warnings, warning] : copy.warnings,
  }
}
