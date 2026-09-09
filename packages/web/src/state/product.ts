import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { SerialQueue } from '@demicodes/utils'
import { apiRequest, readResponse } from '../api/client'
import {
  modelCatalogSchema,
  productStateSchema,
  vendorCatalogSchema,
  type CatalogProvider,
  type ProductState,
  type VendorCatalog,
} from '../api/contracts'

/** One account snapshot; REST polling never replaces agent transcript state. */
export const useProduct = defineStore('product', () => {
  const snapshot = ref<ProductState | null>(null)
  const load = ref<'loading' | 'ready' | 'failed'>('loading')
  const error = ref<string | null>(null)
  const catalogs = ref<Record<string, CatalogProvider[]>>({})
  const vendors = ref<VendorCatalog | null>(null)
  const activeConversationId = ref<string | null>(null)
  const catalog = computed(
    () =>
      catalogs.value[activeConversationId.value ?? ''] ?? catalogs.value[''] ?? [],
  )
  const reads = new SerialQueue()
  const modelRequests = new Map<string, symbol>()
  let controller: AbortController | null = null
  let timer: ReturnType<typeof setTimeout> | null = null
  let etag: string | null = null

  function clearTimer(): void {
    if (timer !== null) {
      clearTimeout(timer)
    }
    timer = null
  }

  async function refresh(): Promise<void> {
    const current = controller
    if (!current) {
      return
    }
    await reads.run(async () => {
      current.signal.throwIfAborted()
      const response = await apiRequest('/state', {
        signal: current.signal,
        headers: etag ? { 'If-None-Match': etag } : {},
        allowNotModified: true,
      })
      if (response.status !== 304) {
        const next = await readResponse(response, productStateSchema)
        current.signal.throwIfAborted()
        snapshot.value = next
        etag = response.headers.get('ETag')
      }
      load.value = 'ready'
      error.value = null
    })
  }

  async function revalidate(): Promise<void> {
    const current = controller
    try {
      await refresh()
      await loadModels()
    } catch (cause) {
      if (!current || current.signal.aborted || controller !== current) {
        return
      }
      error.value = `Could not refresh: ${
        cause instanceof Error ? cause.message : String(cause)
      }`
    }
  }

  async function loadModels(
    conversationId: string | null = activeConversationId.value,
    force = false,
  ): Promise<void> {
    const current = controller
    if (!current) {
      return
    }
    if (conversationId && !snapshot.value?.conversations.some((item) => item.id === conversationId)) {
      conversationId = null
    }
    const key = conversationId ?? ''
    const requestId = Symbol()
    modelRequests.set(key, requestId)
    const query = new URLSearchParams()
    if (conversationId) {
      query.set('conversationId', conversationId)
    }
    if (force) {
      query.set('refresh', 'true')
    }
    const response = await apiRequest(`/models?${query}`, {
      signal: current.signal,
    })
    const next = await readResponse(response, modelCatalogSchema)
    current.signal.throwIfAborted()
    if (modelRequests.get(key) === requestId) {
      modelRequests.delete(key)
      catalogs.value[key] = next.providers
    }
  }

  async function loadVendors(): Promise<void> {
    const current = controller
    if (!current) {
      return
    }
    const response = await apiRequest('/providers/catalog', {
      signal: current.signal,
    })
    const next = await readResponse(response, vendorCatalogSchema)
    current.signal.throwIfAborted()
    vendors.value = next
  }

  async function poll(): Promise<void> {
    const current = controller
    if (!current) {
      return
    }
    try {
      await refresh()
      await loadModels()
    } catch (cause) {
      if (current.signal.aborted) {
        return
      }
      error.value = cause instanceof Error ? cause.message : String(cause)
      if (!snapshot.value) {
        load.value = 'failed'
      }
    } finally {
      if (controller === current && !current.signal.aborted) {
        clearTimer()
        timer = setTimeout(() => void poll(), 3000)
      }
    }
  }

  async function start(): Promise<void> {
    if (controller) {
      return
    }
    controller = new AbortController()
    await poll()
  }

  function stop(): void {
    controller?.abort()
    controller = null
    clearTimer()
    etag = null
    snapshot.value = null
    catalogs.value = {}
    modelRequests.clear()
    vendors.value = null
    activeConversationId.value = null
    load.value = 'loading'
    error.value = null
  }

  return {
    snapshot,
    load,
    error,
    catalogs,
    catalog,
    vendors,
    activeConversationId,
    refresh,
    revalidate,
    loadModels,
    loadVendors,
    start,
    stop,
  }
})
