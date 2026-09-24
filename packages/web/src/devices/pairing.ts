import type { PairingResult } from '@demicodes/web-ui/devices/pairing'
import type { DeviceInstallation } from '@demicodes/web-ui/devices/installation'
import { ApiError, apiRequest, jsonBody, readResponse } from '../api/client'
import { claimedDeviceSchema, type Claim } from '../api/generated/web-api'
import { useProduct } from '../state/product'

// Installer packaging is separate from pairing; these URLs are supplied by the host.
export const deviceInstallation: DeviceInstallation = {
  shellInstallerUrl: `${window.location.origin}/install.sh`,
  powershellInstallerUrl: `${window.location.origin}/install.ps1`,
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
