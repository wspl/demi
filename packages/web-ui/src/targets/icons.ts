import type { Component } from 'vue'
import { Cloud, Monitor } from '@lucide/vue'
import type { TargetKind } from './types'

/** The Cloud's device id, wherever a device is chosen. */
export const CLOUD_DEVICE_ID = 'cloud'

/** The product's one device glyph: a paired device, wherever it shows (`DeviceIcon` adds its state). */
export const DEVICE_ICON: Component = Monitor

/**
 * One glyph per kind of device: the cloud for the Cloud, a monitor for a
 * paired device. A device says which it is by its kind, or by the id the
 * Cloud has wherever a device is chosen.
 */
export function deviceIcon(device: {
  id?: string
  kind?: TargetKind
}): Component {
  return device.kind === 'cloud' || device.id === CLOUD_DEVICE_ID ? Cloud : DEVICE_ICON
}
