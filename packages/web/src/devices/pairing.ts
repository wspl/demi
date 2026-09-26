import { computed, type ComputedRef } from 'vue'
import type { PairingResult } from '@demicodes/web-ui/devices/pairing'
import { deviceInstallationAt, type DeviceInstallation } from '@demicodes/web-ui/devices/installation'
import { ApiError, apiRequest, jsonBody, readResponse } from '../api/client'
import { claimedDeviceSchema, type Claim } from '../api/generated/web-api'
import { useProduct } from '../state/product'

/**
 * The backend's installers, at the public URL the product state names, not at
 * the page's own origin, which in development is Vite's; null until the state
 * has loaded.
 */
export function useDeviceInstallation(): ComputedRef<DeviceInstallation | null> {
  const product = useProduct()
  return computed(() => {
    const publicUrl = product.snapshot?.publicUrl
    return publicUrl ? deviceInstallationAt(publicUrl) : null
  })
}

export async function claimDevice(
  code: string,
  signal?: AbortSignal,
): Promise<PairingResult> {
  try {
    const response = await apiRequest('/devices/claim', {
      method: 'POST',
      signal,
      ...jsonBody({ code } satisfies Claim),
    })
    const { device } = await readResponse(response, claimedDeviceSchema)
    await useProduct().revalidate()
    return {
      ok: true,
      device: { id: device.id, name: device.name },
    }
  } catch (error) {
    if (
      error instanceof ApiError &&
      (error.code === 'invalid_code' || error.code === 'rate_limited')
    ) {
      return {
        ok: false,
        code: error.code,
      }
    }
    throw error
  }
}
