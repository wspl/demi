import type { PairingResult } from '@demicodes/web-ui/devices/pairing'
import type { DeviceInstallation } from '@demicodes/web-ui/devices/installation'
import { useResources } from './resources'

/** The prototype's pairing: an installer served from this origin, and a claim that adds a device at once. */
export const deviceInstallation: DeviceInstallation = {
  shellInstallerUrl: `${window.location.origin}/install.sh`,
  powershellInstallerUrl: `${window.location.origin}/install.ps1`,
}

export async function claimDevice(_code: string): Promise<PairingResult> {
  const resources = useResources()
  const device = {
    id: crypto.randomUUID(),
    name: `host-${resources.devices.length + 1}`,
    online: true,
    platform: 'linux' as const,
    home: '/home/demo',
  }
  resources.devices.push(device)
  return { ok: true, device }
}
