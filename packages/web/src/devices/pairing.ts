import { z } from 'zod'
import type { PairingResult } from '@demicodes/web-ui/devices/pairing'
import type { DeviceInstallation } from '@demicodes/web-ui/devices/installation'
import { ApiError, apiRequest, jsonBody, readResponse } from '../api/client'
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
      ...jsonBody({ code }),
    })
    const result = await readResponse(
      response,
      z.object({
        device: z.object({
          id: z.string(),
          name: z.string(),
        }),
      }),
    )
    await useProduct().revalidate()
    return {
      ok: true,
      device: result.device,
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
