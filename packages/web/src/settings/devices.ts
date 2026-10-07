import { defineStore } from 'pinia'
import { useSession } from '../auth/session'
import { computed, onScopeDispose, ref, watch } from 'vue'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { useProduct } from '../state/product'
import { ApiError, apiRequest, jsonBody, readResponse } from '../api/client'
import {
  cloudResetAnswerSchema,
  revokedDeviceSchema,
  type CloudReset,
} from '../api/generated/web-api'

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
  // The product state carries the Cloud's status; there is none only before
  // the channel's first snapshot arrives.
  const cloud = computed(() => {
    const status = product.snapshot?.cloud
    if (!status) {
      return null
    }
    return {
      state: status.state,
      operationId: status.operation?.id ?? null,
      phase: status.operation?.phase ?? null,
      error: status.error ?? status.operation?.error ?? null,
      volumes: status.volumes,
      limits: status.limits,
      newerImage: status.newerImage,
    }
  })
  async function revoke(id: string): Promise<void> {
    if (revoking.value.includes(id)) {
      return
    }
    const current = lifetime
    revoking.value.push(id)
    try {
      const response = await apiRequest(`/devices/${encodeURIComponent(id)}`, {
        method: 'DELETE',
        signal: current.signal,
      })
      // The device's projects went with it; the channel brings the lists
      // without them.
      await readResponse(response, revokedDeviceSchema)
    } catch (error) {
      if (!current.signal.aborted) {
        reportError('Could Not Revoke Device', error, { userVisible: true })
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
      const response = await apiRequest('/cloud/reset', {
        method: 'POST',
        signal: current.signal,
        ...jsonBody({ operationId } satisfies CloudReset),
      })
      await readResponse(response, cloudResetAnswerSchema)
      current.signal.throwIfAborted()
      reset.value = { status: 'idle' }
    } catch (error) {
      if (!current.signal.aborted) {
        // The backend's refusal is a sentence for the reader; anything else
        // (a proxy's page, a dropped connection) is said in words of our own,
        // and its own text goes to the console.
        const refused = error instanceof ApiError && error.code !== null
        if (!refused) {
          console.error('[demi] Cloud reset failed:', error)
        }
        reset.value = {
          status: 'failed',
          message: refused ? error.message : 'The reset could not start. Check your connection and try again.',
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
  return {
    cloud,
    reset,
    revoking,
    revoke,
    resetCloud,
  }
})
