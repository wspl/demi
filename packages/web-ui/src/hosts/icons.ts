import type { Component } from 'vue'
import { Cloud, Monitor, RadioTower } from '@lucide/vue'

/** The managed workspace's host id, wherever a device is chosen. */
export const CLOUD_HOST_ID = 'cloud'

/** The glyph of an expose wherever one shows: the session tools button and the work panel tab that frames it. */
export const EXPOSE_ICON: Component = RadioTower

/** One glyph per kind of host: the cloud for the managed workspace, a monitor for a device, unless the host names its own. */
export function hostIcon(host: {
  id: string;
  icon?: Component
}): Component {
  return host.icon ?? (host.id === CLOUD_HOST_ID ? Cloud : Monitor)
}
