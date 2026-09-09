export type DeviceSystem = 'linux' | 'macos' | 'windows'

/** Deployment-provided installers carry their backend configuration. */
export interface DeviceInstallation {
  shellInstallerUrl: string
  powershellInstallerUrl: string
}

export const deviceSystems = [
  { value: 'linux', label: 'Linux' },
  { value: 'macos', label: 'macOS' },
  { value: 'windows', label: 'Windows' },
] as const

export function deviceInstallCommand(
  installation: DeviceInstallation,
  system: DeviceSystem
): string {
  if (system === 'windows') {
    const quote = (value: string) => `'${value.replaceAll("'", "''")}'`
    return `irm ${quote(installation.powershellInstallerUrl)} | iex`
  }
  const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`
  return `curl --proto '=https' --tlsv1.2 -sSf ${quote(installation.shellInstallerUrl)} | sh`
}
