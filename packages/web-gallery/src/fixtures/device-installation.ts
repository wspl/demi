import type { DeviceInstallation } from '@demicodes/web-ui/devices/installation'

/** Proposed installer endpoints for the visual prototype, not published downloads. */
export const demoDeviceInstallation: DeviceInstallation = {
  backendUrl: 'https://demi.example.com',
  shellInstallerUrl: 'https://demi.example.com/install.sh',
  powershellInstallerUrl: 'https://demi.example.com/install.ps1',
}
