import type { Component } from 'vue'
import { Cloud, Monitor } from '@lucide/vue'
import type { HostKind } from './types'

/** The managed workspace's host id, wherever a device is chosen. */
export const CLOUD_HOST_ID = 'cloud'

/** The product's one device glyph: a host that is a device, wherever it shows (`DeviceIcon` adds its state). */
export const DEVICE_ICON: Component = Monitor

/**
 * One glyph per kind of host: the cloud for the managed workspace, a monitor
 * for a device. A host says which it is by its kind, or by the id the Cloud
 * has wherever a device is chosen.
 */
export function hostIcon(host: {
  id?: string
  kind?: HostKind
}): Component {
  return host.kind === 'cloud' || host.id === CLOUD_HOST_ID ? Cloud : DEVICE_ICON
}
