import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { SerialQueue } from '@demicodes/utils'
import { reportError } from '@demicodes/web-ui/infra/errors'
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
  // A poll that fails while a snapshot is on screen is one toast per outage, not one per tick.
  let refreshFailed = false
  const catalogs = ref<Record<string, CatalogProvider[]>>({})
  const vendors = ref<VendorCatalog | null>(null)
  const modelErrors = ref<Record<string, string>>({})
  const vendorError = ref<string | null>(null)
  const vendorLoad = computed(() =>
    vendors.value !== null ? 'ready' : vendorError.value ? 'failed' : 'loading',
  )
  const catalogLoad = computed(() => {
    const key = activeConversationId.value ?? ''
    if (catalogs.value[key] || catalogs.value['']) {
      return 'ready'
    }
    return modelErrors.value[key] || modelErrors.value['']
      ? 'failed'
      : 'loading'
  })
  const activeConversationId = ref<string | null>(null)
  function catalogFor(conversationId: string | null) {
    return catalogs.value[conversationId ?? ''] ?? catalogs.value[''] ?? []
  }
  const catalog = computed(() => catalogFor(activeConversationId.value))
  const reads = new SerialQueue()
  const modelRequests = new Map<string, symbol>()
  let vendorRequest: Promise<void> | null = null
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
    if (!snapshot.value) {
      load.value = 'loading'
    }
    try {
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
        refreshFailed = false
      })
    } catch (cause) {
      if (
        controller === current &&
        !current.signal.aborted &&
        !snapshot.value
      ) {
        load.value = 'failed'
      }
      throw cause
    }
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
      reportError('Could not refresh', cause, { userVisible: true })
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
    if (
      conversationId &&
      !snapshot.value?.conversations.some((item) => item.id === conversationId)
    ) {
      conversationId = null
    }
    const key = conversationId ?? ''
    const requestId = Symbol()
    modelRequests.set(key, requestId)
    delete modelErrors.value[key]
    const query = new URLSearchParams()
    if (conversationId) {
      query.set('conversationId', conversationId)
    }
    if (force) {
      query.set('refresh', 'true')
    }
    try {
      const response = await apiRequest(`/models?${query}`, {
        signal: current.signal,
      })
      const next = await readResponse(response, modelCatalogSchema)
      current.signal.throwIfAborted()
      if (modelRequests.get(key) === requestId) {
        catalogs.value[key] = next.providers
        delete modelErrors.value[key]
      }
    } catch (cause) {
      if (!current.signal.aborted && modelRequests.get(key) === requestId) {
        modelErrors.value[key] =
          cause instanceof Error ? cause.message : String(cause)
      }
      throw cause
    } finally {
      if (modelRequests.get(key) === requestId) {
        modelRequests.delete(key)
      }
    }
  }

  async function loadVendors(): Promise<void> {
    const current = controller
    if (!current || vendors.value !== null) {
      return
    }
    if (vendorRequest) {
      return vendorRequest
    }
    vendorError.value = null
    const request = (async () => {
      try {
        const response = await apiRequest('/providers/catalog', {
          signal: current.signal,
        })
        const next = await readResponse(response, vendorCatalogSchema)
        current.signal.throwIfAborted()
        vendors.value = next
      } catch (cause) {
        if (!current.signal.aborted) {
          vendorError.value =
            cause instanceof Error ? cause.message : String(cause)
        }
        throw cause
      }
    })()
    vendorRequest = request
    try {
      await request
    } finally {
      if (vendorRequest === request) {
        vendorRequest = null
      }
    }
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
      if (!snapshot.value) {
        load.value = 'failed'
      } else if (!refreshFailed) {
        refreshFailed = true
        reportError('Could not refresh', cause, { userVisible: true })
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
    vendorRequest = null
    vendorError.value = null
    modelErrors.value = {}
    activeConversationId.value = null
    load.value = 'loading'
    refreshFailed = false
  }

  return {
    snapshot,
    load,
    catalogs,
    catalog,
    catalogFor,
    vendors,
    vendorLoad,
    catalogLoad,
    activeConversationId,
    refresh,
    revalidate,
    loadModels,
    loadVendors,
    start,
    stop,
  }
})
