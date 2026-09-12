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
  const modelSnapshot = ref<{ key: string; providers: CatalogProvider[]; checkedAt: number } | null>(null)
  const vendors = ref<VendorCatalog | null>(null)
  const modelError = ref<{ key: string; retryAt: number } | null>(null)
  const vendorError = ref<string | null>(null)
  const vendorLoad = computed(() =>
    vendors.value !== null ? 'ready' : vendorError.value ? 'failed' : 'loading',
  )
  const catalogLoad = computed(() => modelSnapshot.value?.key === catalogKey.value
    ? 'ready' : modelError.value?.key === catalogKey.value ? 'failed' : 'loading')
  const activeConversationId = ref<string | null>(null)
  const catalogKey = computed(() => JSON.stringify((snapshot.value?.providers ?? []).map(provider => {
    const { details, error: _error, ...config } = provider
    return { ...config, account: details?.active?.credentialId ?? null }
  })))
  const catalog = computed(() => modelSnapshot.value?.key === catalogKey.value
    ? modelSnapshot.value.providers : [])
  const reads = new SerialQueue()
  let modelRequest: { key: string; promise: Promise<void>; controller: AbortController } | null = null
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
      })
    } catch (cause) {
      // A refresh that fails while a snapshot is on screen is not told: the
      // snapshot stays, the next poll tries again, and a lost connection is
      // the session's to reconnect. Only the first load has nothing to keep.
      if (controller === current && !current.signal.aborted && !snapshot.value) {
        load.value = 'failed'
      }
      throw cause
    }
  }

  async function revalidate(refreshModels = false): Promise<void> {
    if (refreshModels)
      clearModels()
    try {
      await refresh()
      await loadModels(refreshModels)
    } catch {
      // A failed refresh keeps the snapshot; a model catalog failure is the
      // composer's state.
    }
  }

  function clearModels(): void {
    modelRequest?.controller.abort()
    modelRequest = null
    modelSnapshot.value = null
    modelError.value = null
  }

  async function loadModels(force = false): Promise<void> {
    const current = controller
    if (!current)
      return
    const key = catalogKey.value
    if (modelRequest?.key === key)
      return modelRequest.promise
    if (!force && modelSnapshot.value?.key === key && Date.now() - modelSnapshot.value.checkedAt < 60_000)
      return
    if (!force && modelError.value?.key === key && Date.now() < modelError.value.retryAt)
      return
    modelRequest?.controller.abort()
    const requestController = new AbortController()
    const signal = AbortSignal.any([current.signal, requestController.signal])
    modelError.value = null
    const request = (async () => {
      try {
        const response = await apiRequest(force ? '/models?refresh=true' : '/models', {
          signal,
        })
        const next = await readResponse(response, modelCatalogSchema)
        signal.throwIfAborted()
        if (key === catalogKey.value)
          modelSnapshot.value = { key, providers: next.providers, checkedAt: Date.now() }
      } catch (cause) {
        if (!signal.aborted && key === catalogKey.value) {
          modelError.value = {
            key,
            retryAt: Date.now() + 60_000,
          }
        }
        throw cause
      }
    })()
    modelRequest = { key, promise: request, controller: requestController }
    try {
      await request
    } finally {
      if (modelRequest?.promise === request)
        modelRequest = null
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
      // Model discovery must not hold initial app or conversation restoration.
      // loadModels records its own failure and keeps the last usable snapshot.
      void loadModels().catch(() => {})
    } catch {
      // refresh records its own failure.
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
    clearModels()
    vendors.value = null
    vendorRequest = null
    vendorError.value = null
    activeConversationId.value = null
    load.value = 'loading'
  }

  return {
    snapshot,
    load,
    catalog,
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
