import { t } from './locales/text'
import { defineStore } from 'pinia'
import { useSession } from '../auth/session'
import { computed, onScopeDispose, ref, watch } from 'vue'
import { SerialQueue } from '@demicodes/utils'
import type { HeadlineText } from '@demicodes/web-ui/ui/ui-text'
import { errorCode, reportError, shownError, type ShownError } from '@demicodes/web-ui/infra/errors'
import { useProduct } from '../state/product'
import { cloudDeviceIn } from '../state/hosts'
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
  /**
   * The device changes on their way, by device: each device's go one at a
   * time in order, as the backend answers each with the whole device.
   */
  const writes = new SerialQueue()
  /** Why the last Cloud reset was refused; the dialog shows the reset under way from its own request. */
  const resetError = ref<ShownError | null>(null)
  // The product state carries the Cloud's status; there is none only before
  // the channel's first snapshot arrives. Its device, and what its runner
  // reported, is there once the Cloud's first use made it.
  const cloud = computed(() => {
    const status = product.snapshot?.cloud
    if (!status) {
      return null
    }
    const device = cloudDeviceIn(product.snapshot?.devices)
    return {
      state: status.state,
      operationId: status.operation?.id ?? null,
      phase: status.operation?.phase ?? null,
      error: status.error ?? status.operation?.error ?? null,
      volumes: status.volumes,
      newerImage: status.newerImage,
      deviceId: device?.id ?? null,
      report: { os: device?.os ?? null, runnerVersion: device?.runnerVersion ?? null },
    }
  })
  async function revoke(id: string): Promise<void> {
    const current = lifetime
    try {
      const response = await apiRequest(`/devices/${encodeURIComponent(id)}`, {
        method: 'DELETE',
        signal: current.signal,
      })
      // The device's projects went with it; the channel brings the lists
      // without them.
      await readResponse(response, revokedDeviceSchema)
    } catch (error) {
      // A device another page revoked meanwhile is revoked: that is done.
      if (!current.signal.aborted && errorCode(error) !== 'device_not_found') {
        reportError(t('devices.revokeFailed'), error, { userVisible: true })
      }
    }
  }

  /**
   * Changes a paired device as `body` says, at once, and sends the change
   * after the earlier ones of the same device; the change lands with the device the backend
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
      .run(id, async () => {
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
    return change(id, { name: name.trim() }, t('devices.renameFailed'))
  }

  /** Sets a paired device's route, which every page of the user's follows. */
  function setRoute(id: string, route: DeviceRoute): Promise<void> {
    return change(id, { route }, t('devices.routeFailed'))
  }

  async function resetCloud(operationId: string): Promise<void> {
    const current = lifetime
    resetError.value = null
    try {
      const response = await apiRequest('/cloud/reset', {
        method: 'POST',
        signal: current.signal,
        ...jsonBody({ operationId } satisfies CloudReset),
      })
      await readResponse(response, cloudResetAnswerSchema)
    } catch (error) {
      if (!current.signal.aborted) {
        // The backend's refusal is a sentence for the reader; anything else
        // (a proxy's page, a dropped connection) is said in words of our own,
        // and its own text goes to the console.
        const refused = error instanceof ApiError && error.code !== null
        if (!refused) {
          console.error('[demi] Cloud reset failed:', error)
        }
        resetError.value = shownError(
          error,
          refused ? undefined : t('devices.resetNotStarted'),
        )
      }
    }
  }

  watch(
    () => useSession().user?.id,
    () => {
      lifetime.abort()
      lifetime = new AbortController()
      resetError.value = null
    },
  )
  onScopeDispose(() => lifetime.abort())
  return {
    cloud,
    resetError,
    revoke,
    rename,
    setRoute,
    resetCloud,
  }
})
