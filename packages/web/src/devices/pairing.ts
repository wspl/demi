import { computed, type ComputedRef } from 'vue'
import type { PairingResult } from '@demicodes/web-ui/devices/pairing'
import { ApiError, apiRequest, jsonBody, readResponse } from '../api/client'
import { deviceAnswerSchema, type Claim, type InstallCommands } from '../api/generated/web-api'
import { useProduct } from '../state/product'

/** The refusals of a pairing code the dialog words itself. */
const PAIRING_REFUSALS = ['invalid_code', 'rate_limited'] as const

/** The install commands the product state carries; null until it has loaded. */
export function useInstallCommands(): ComputedRef<InstallCommands | null> {
  const product = useProduct()
  return computed(() => product.snapshot?.installCommands ?? null)
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
    // The device list shows it once the channel brings the devices.
    const { device } = await readResponse(response, deviceAnswerSchema)
    return {
      ok: true,
      device: { id: device.id, name: device.name },
    }
  } catch (error) {
    if (error instanceof ApiError) {
      const code = PAIRING_REFUSALS.find((refusal) => refusal === error.code)
      if (code !== undefined) {
        return { ok: false, code, requestId: error.requestId }
      }
    }
    throw error
  }
}
