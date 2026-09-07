export type DeviceSystem = 'linux' | 'macos' | 'windows'

/** Deployment-provided installer endpoints; the UI does not publish installers. */
export interface DeviceInstallation {
  backendUrl: string
  shellInstallerUrl: string
  powershellInstallerUrl: string
}

export const deviceSystems = [
  { value: 'linux', label: 'Linux' },
  { value: 'macos', label: 'macOS' },
  { value: 'windows', label: 'Windows' },
] as const

export function deviceInstallCommand(installation: DeviceInstallation, system: DeviceSystem): string {
  if (system === 'windows') {
    const quote = (value: string) => `'${value.replaceAll("'", "''")}'`
    return `& ([scriptblock]::Create((Invoke-RestMethod ${quote(installation.powershellInstallerUrl)}))) -Backend ${quote(installation.backendUrl)}`
  }
  const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`
  return `curl -fsSL ${quote(installation.shellInstallerUrl)} | sh -s -- --backend ${quote(installation.backendUrl)}`
}
