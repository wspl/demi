import { defineStore } from 'pinia'
import { useSession } from '../auth/session'
import { computed, onScopeDispose, ref, watch } from 'vue'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { useProduct } from '../state/product'
import { apiRequest, jsonBody } from '../api/client'
import { renewExpose, removeExpose } from '../api/exposes'

export const useDeviceSettings = defineStore('device-settings', () => {
  const product = useProduct()
  let lifetime = new AbortController()
  const revoking = ref<string[]>([])
  const exposePending = ref<string[]>([])
  const reset = ref<
    | { status: 'idle' | 'pending' }
    | {
        status: 'failed'
        message: string
      }
  >({ status: 'idle' })
  const cloud = computed(() =>
    product.snapshot?.cloud
      ? {
          state: product.snapshot.cloud.state,
          phase: product.snapshot.cloud.operation?.phase ?? null,
          error:
            product.snapshot.cloud.error ??
            product.snapshot.cloud.operation?.error ??
            null,
          ...product.snapshot.cloud.limits,
        }
      : null,
  )
  const exposes = computed(() => product.snapshot?.exposes ?? [])
  const exposeDomain = computed(() => product.snapshot?.exposeDomain ?? null)

  /** Shared body of renew and remove: one request per expose, then a fresh snapshot. */
  async function exposeWrite(
    id: string,
    run: (signal: AbortSignal) => Promise<void>,
    couldNot: string,
  ): Promise<void> {
    if (exposePending.value.includes(id)) {
      return
    }
    const current = lifetime
    exposePending.value.push(id)
    try {
      await run(current.signal)
      await product.revalidate()
    } catch (error) {
      if (!current.signal.aborted) {
        reportError(couldNot, error, { userVisible: true })
      }
    } finally {
      if (current === lifetime) {
        exposePending.value = exposePending.value.filter((value) => value !== id)
      }
    }
  }

  function renewExposeAction(id: string): Promise<void> {
    return exposeWrite(
      id,
      (signal) => renewExpose(id, signal),
      'Could not renew expose',
    )
  }

  function removeExposeAction(id: string): Promise<void> {
    return exposeWrite(
      id,
      (signal) => removeExpose(id, signal),
      'Could not remove expose',
    )
  }

  async function revoke(id: string): Promise<void> {
    if (revoking.value.includes(id)) {
      return
    }
    const current = lifetime
    revoking.value.push(id)
    try {
      await apiRequest(`/devices/${encodeURIComponent(id)}`, {
        method: 'DELETE',
        signal: current.signal,
      })
      await product.revalidate()
    } catch (error) {
      if (!current.signal.aborted) {
        reportError('Could not revoke device', error, { userVisible: true })
      }
    } finally {
      if (current === lifetime) {
        revoking.value = revoking.value.filter((value) => value !== id)
      }
    }
  }

  async function resetCloud(operationId: string): Promise<void> {
    if (reset.value.status === 'pending') {
      return
    }
    const current = lifetime
    reset.value = { status: 'pending' }
    try {
      await apiRequest('/cloud/reset', {
        method: 'POST',
        signal: current.signal,
        ...jsonBody({ operationId }),
      })
      current.signal.throwIfAborted()
      await product.revalidate()
      current.signal.throwIfAborted()
      reset.value = { status: 'idle' }
    } catch (error) {
      if (!current.signal.aborted) {
        reset.value = {
          status: 'failed',
          message: error instanceof Error ? error.message : String(error),
        }
      }
    }
  }

  watch(
    () => useSession().user?.id,
    () => {
      lifetime.abort()
      lifetime = new AbortController()
      revoking.value = []
      exposePending.value = []
      reset.value = { status: 'idle' }
    },
  )
  onScopeDispose(() => lifetime.abort())
  return {
    cloud,
    reset,
    revoking,
    revoke,
    resetCloud,
    exposes,
    exposeDomain,
    exposePending,
    renewExpose: renewExposeAction,
    removeExpose: removeExposeAction,
  }
})
