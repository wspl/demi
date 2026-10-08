import { h, type FunctionalComponent } from 'vue'
import DeviceIcon from './DeviceIcon.vue'
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
