import { h, type Component, type FunctionalComponent } from 'vue'
import DeviceIcon from './DeviceIcon.vue'
import { hostIcon } from '../hosts/icons'
import type { HostKind } from '../hosts/types'
import type { DeviceState } from './state'

/**
 * A device with its state as an icon a row takes by component, such as a
 * menu row's: `DeviceIcon` for each state, so a device looks the same
 * wherever it is drawn.
 */
export const DEVICE_GLYPHS: Record<DeviceState, FunctionalComponent> = {
  online: () => h(DeviceIcon, { state: 'online' }),
  updating: () => h(DeviceIcon, { state: 'updating' }),
  offline: () => h(DeviceIcon, { state: 'offline' }),
}

/** A host as an icon a row takes by component: the Cloud's glyph, or a device with its state. */
export function hostGlyph(host: { kind: HostKind; state: DeviceState }): Component {
  return host.kind === 'cloud' ? hostIcon(host) : DEVICE_GLYPHS[host.state]
}
