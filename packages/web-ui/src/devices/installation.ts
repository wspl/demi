import type { ClientPlatform } from '@demicodes/utils'
import type { SegmentedOption } from '../ui/Segmented.vue'

export type DeviceSystem = 'linux' | 'macos' | 'windows'

/** Where a device fetches the runner's installer for its system. */
export interface DeviceInstallation {
  shellInstallerUrl: string
  powershellInstallerUrl: string
}

export const deviceSystems: readonly SegmentedOption<DeviceSystem>[] = [
  { value: 'linux', label: 'Linux' },
  { value: 'macos', label: 'macOS' },
  { value: 'windows', label: 'Windows' },
]

/**
 * The system a device added from this browser most likely runs: the
 * browser's own, since a person usually adds the computer they are using.
 */
export function deviceSystemOf(platform: ClientPlatform): DeviceSystem {
  if (platform === 'mac') {
    return 'macos'
  }
  return platform === 'windows' ? 'windows' : 'linux'
}

/**
 * The installers of the backend whose public URL is `publicUrl`, the URL its
 * runners connect to: the backend serves them at that URL's origin
 * (`web-api.md` § Resource index), wherever the page itself is served from.
 */
export function deviceInstallationAt(publicUrl: string): DeviceInstallation {
  return {
    shellInstallerUrl: new URL('/install.sh', publicUrl).href,
    powershellInstallerUrl: new URL('/install.ps1', publicUrl).href,
  }
}

/**
 * The command that installs and starts the runner on `system`. curl fetches
 * the installer only over the scheme of its URL, and over TLS 1.2 or newer
 * for `https`, so a backend served over `http`, as in development, is
 * reached too.
 */
export function deviceInstallCommand(
  installation: DeviceInstallation,
  system: DeviceSystem
): string {
  if (system === 'windows') {
    const quote = (value: string) => `'${value.replaceAll("'", "''")}'`
    return `irm ${quote(installation.powershellInstallerUrl)} | iex`
  }
  const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`
  const scheme = new URL(installation.shellInstallerUrl).protocol.slice(0, -1)
  const tls = scheme === 'https' ? ' --tlsv1.2' : ''
  return `curl --proto '=${scheme}'${tls} -sSf ${quote(installation.shellInstallerUrl)} | sh`
}
