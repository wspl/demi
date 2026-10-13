import { createLucideIcon, type IconNode } from '@lucide/vue'
import type { ClientPlatform } from '@demicodes/utils'
import { siApple, siLinux } from 'simple-icons'
import type { SegmentedOption } from '../ui/Segmented.vue'

export type DeviceSystem = 'linux' | 'macos' | 'windows'

/**
 * How a paired device's runner is started again: the command its
 * installation's launcher takes, typed on the device's `system`.
 */
export interface DeviceStart {
  command: string
  system: DeviceSystem
}

/**
 * The command that installs and starts the runner on each system, as the
 * backend composes it from the URL its runners connect to.
 */
export type DeviceInstallCommands = Readonly<Record<DeviceSystem, string>>

/** A filled 24-unit mark drawn at the size and colour of Lucide's icons. */
function systemMark(name: string, paths: readonly string[]) {
  const node: IconNode = paths.map((d) => ['path', { d, fill: 'currentColor', stroke: 'none' }])
  return createLucideIcon(name, node)
}

// Simple Icons dropped the Windows logo at Microsoft's request; it is four squares.
const windowsMark = ['M0 0h11.4v11.4H0z', 'M12.6 0H24v11.4H12.6z', 'M0 12.6h11.4V24H0z', 'M12.6 12.6H24V24H12.6z']

export const deviceSystems: readonly SegmentedOption<DeviceSystem>[] = [
  { value: 'linux', label: 'Linux', icon: systemMark('linux', [siLinux.path]) },
  { value: 'macos', label: 'macOS', icon: systemMark('apple', [siApple.path]) },
  { value: 'windows', label: 'Windows', icon: systemMark('windows', windowsMark) },
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
