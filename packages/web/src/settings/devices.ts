import { defineStore } from 'pinia'
import { useSession } from '../auth/session'
import { computed, onScopeDispose, ref, watch } from 'vue'
import { SerialQueue } from '@demicodes/utils'
import type { HeadlineText } from '@demicodes/web-ui/ui/ui-text'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { useProduct } from '../state/product'
import { ApiError, apiRequest, jsonBody, readResponse } from '../api/client'
import {
  cloudResetAnswerSchema,
  deviceAnswerSchema,
  revokedDeviceSchema,
  type CloudReset,
  type ChangeDevice,
  type DeviceRoute,
} from '../api/generated/web-api'

export const useDeviceSettings = defineStore('device-settings', () => {
  const product = useProduct()
  let lifetime = new AbortController()
  const revoking = ref<string[]>([])
  /** The device changes on their way, which go one at a time in order. */
  const writes = new SerialQueue()
  const reset = ref<
    | { status: 'idle' | 'pending' }
    | {
        status: 'failed'
        message: string
      }
  >({ status: 'idle' })
  // The product state carries the Cloud's status; there is none only before
  // the channel's first snapshot arrives. Its device, and what its runner
  // reported, is there once the Cloud's first use made it.
  const cloud = computed(() => {
    const status = product.snapshot?.cloud
    if (!status) {
      return null
    }
    const device = product.snapshot?.devices.find((candidate) => candidate.kind === 'managed')
    return {
      state: status.state,
      operationId: status.operation?.id ?? null,
      phase: status.operation?.phase ?? null,
      error: status.error ?? status.operation?.error ?? null,
      volumes: status.volumes,
      limits: status.limits,
      newerImage: status.newerImage,
      deviceId: device?.id ?? null,
      report: { os: device?.os ?? null, runnerVersion: device?.runnerVersion ?? null },
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

  /**
   * Changes a paired device as `body` says, at once, and sends the change
   * after the earlier ones; the change lands with the device the backend
   * answers, the channel brings it to every other page, and a refusal gives
   * the device back with a toast titled `failed`.
   */
  function change(
    id: string,
    body: { name: string } | { route: DeviceRoute },
    failed: HeadlineText,
  ): Promise<void> {
    const shown = product.change('devices', (state) => ({
      ...state,
      devices: state.devices.map((device) => device.id === id ? { ...device, ...body } : device),
    }))
    const current = lifetime
    return writes
      .run(async () => {
        current.signal.throwIfAborted()
        shown.send()
        const response = await apiRequest(`/devices/${encodeURIComponent(id)}`, {
          method: 'PATCH',
          signal: current.signal,
          ...jsonBody(body satisfies ChangeDevice),
        })
        const { device } = await readResponse(response, deviceAnswerSchema)
        shown.land((read) => ({
          type: 'devices',
          devices: read.devices.map((candidate) => candidate.id === device.id ? device : candidate),
        }))
      })
      .catch((error) => {
        shown.drop()
        if (!current.signal.aborted) {
          reportError(failed, error, { userVisible: true })
        }
      })
  }

  /** Gives a paired device a new name, as the backend keeps it: without the spaces around it. */
  function rename(id: string, name: string): Promise<void> {
    return change(id, { name: name.trim() }, 'Could Not Rename Device')
  }

  /** Sets a paired device's route, which every page of the user's follows. */
  function setRoute(id: string, route: DeviceRoute): Promise<void> {
    return change(id, { route }, 'Could Not Change the Route')
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
    rename,
    setRoute,
    resetCloud,
  }
})
