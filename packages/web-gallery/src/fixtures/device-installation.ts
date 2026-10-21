import type { DeviceInstallCommands, DeviceStart, DeviceSystem } from '@demicodes/web-ui/devices/installation'
import type { DeviceReport } from '@demicodes/web-ui/devices/report'

/** The install commands the demo backend composes, for the visual prototype, not published downloads. */
export const demoInstallCommands: DeviceInstallCommands = {
  linux: "curl --proto '=https' --tlsv1.2 -sSf 'https://demi.example.com/install.sh' | sh",
  macos: "curl --proto '=https' --tlsv1.2 -sSf 'https://demi.example.com/install.sh' | sh",
  windows: "irm 'https://demi.example.com/install.ps1' | iex",
}

/** A development backend's install commands, which fetch its installers over plain HTTP. */
export const developmentInstallCommands: DeviceInstallCommands = {
  linux: "curl --proto '=http' -sSf 'http://192.168.5.2:3271/install.sh' | sh",
  macos: "curl --proto '=http' -sSf 'http://192.168.5.2:3271/install.sh' | sh",
  windows: "irm 'http://192.168.5.2:3271/install.ps1' | iex",
}

/** The installation the demo backend's installers make: named after the backend's host and a short hash of its URL. */
const demoInstallationId = 'demi.example.com-748c'

/** How a device paired with the demo backend starts its runner again, as the backend names it. */
export function demoDeviceStart(system: DeviceSystem): DeviceStart {
  const command =
    system === 'windows'
      ? `powershell -ExecutionPolicy Bypass -File "$env:USERPROFILE\\.demi\\runner\\${demoInstallationId}\\run.ps1" start`
      : `~/.demi/runner/${demoInstallationId}/run start`
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
