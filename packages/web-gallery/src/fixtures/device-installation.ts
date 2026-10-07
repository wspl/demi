import { deviceInstallationAt, type DeviceStart, type DeviceSystem } from '@demicodes/web-ui/devices/installation'
import type { DeviceReport } from '@demicodes/web-ui/devices/report'

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

/** What a demo device's runner reports of its machine and itself, by its system; `release` is the runner's. */
export function demoDeviceReport(system: DeviceSystem, release = '0.1.16'): DeviceReport {
  const os = {
    macos: { name: 'macOS 26.5', arch: 'aarch64' },
    linux: { name: 'Ubuntu 26.04', arch: 'x86_64' },
    windows: { name: 'Windows 11 Pro (build 26100)', arch: 'x86_64' },
  }[system]
  return { os, runnerVersion: release }
}
