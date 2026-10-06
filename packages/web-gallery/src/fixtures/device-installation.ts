import { deviceInstallationAt, type DeviceStart, type DeviceSystem } from '@demicodes/web-ui/devices/installation'

/** Proposed installer endpoints for the visual prototype, not published downloads. */
export const demoDeviceInstallation = deviceInstallationAt('https://demi.example.com')

/** A development backend's installers, which it serves over plain HTTP. */
export const developmentDeviceInstallation = deviceInstallationAt('http://192.168.5.2:3271')

/** The installation the demo backend's installers make: the SHA-256 of its URL. */
const demoInstallationId = '748c25663f69ee8fdc3510acd09c38e08112c001195c9340fe2aadec220081ce'

/** How a device paired with the demo backend starts its runner again, as the backend names it. */
export function demoDeviceStart(system: DeviceSystem): DeviceStart {
  const command =
    system === 'windows'
      ? `powershell -ExecutionPolicy Bypass -File "$env:USERPROFILE\\.demi\\instances\\${demoInstallationId}\\run.ps1" start`
      : `~/.demi/instances/${demoInstallationId}/run start`
  return { command, system }
}
