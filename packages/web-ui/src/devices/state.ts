import { t } from './locales/text'
import type { StatusDotTone } from '../ui/StatusDot.vue'
import type { TitleText } from '../ui/ui-text'

/**
 * Whether a device's runner serves it: online while it is connected,
 * updating while it replaces itself with its server's release, offline
 * otherwise.
 */
export type DeviceState = 'online' | 'updating' | 'offline'

/** How a device's state reads beside it. */
export const DEVICE_STATE_LABEL: Record<DeviceState, TitleText> = {
  get online() {
    return t('state.online')
  },
  get updating() {
    return t('state.updating')
  },
  get offline() {
    return t('state.offline')
  },
}

/** The tone of a device's state dot. */
export const DEVICE_STATE_TONE: Record<DeviceState, StatusDotTone & ('success' | 'warning' | 'muted')> = {
  online: 'success',
  updating: 'warning',
  offline: 'muted',
}
