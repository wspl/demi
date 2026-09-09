import { defineStore } from 'pinia'
import { useSession } from '../auth/session'
import { computed, onScopeDispose, ref, watch } from 'vue'
import { showToast } from '@demicodes/web-ui/infra/toast'
import { useProduct } from '../state/product'
import { apiRequest, jsonBody } from '../api/client'

export const useDeviceSettings = defineStore('device-settings', () => {
  const product = useProduct()
  let lifetime = new AbortController()
  const revoking = ref<string[]>([])
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
        showToast({
          title: 'Could not revoke device',
          message: error instanceof Error ? error.message : String(error),
          tone: 'danger',
        })
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
      reset.value = { status: 'idle' }
    },
  )
  onScopeDispose(() => lifetime.abort())
  return { cloud, reset, revoking, revoke, resetCloud }
})
