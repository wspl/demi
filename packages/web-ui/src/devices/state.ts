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

/**
 * A device's state, with how much of the runner release it moves to its
 * runner has downloaded while that download moves (`crates/runner/src/update.rs`): ahead while it is online, or after its server switched while it
 * updates; null or absent while none moves.
 */
export interface DeviceStatus {
  state: DeviceState
  downloadPercent?: number | null
}

/**
 * How a device's state reads beside it, with its download: *Downloading
 * update · 40%* while it is online, *Updating · 26%* while it updates, and
 * its state alone while no download moves.
 */
export function deviceStatusLabel(status: DeviceStatus): TitleText {
  const percent = status.downloadPercent
  if (percent == null || status.state === 'offline') {
    return DEVICE_STATE_LABEL[status.state]
  }
  return status.state === 'online'
    ? t('state.downloading', { percent })
    : t('state.updatingPercent', { percent })
}

/** The tone of a device's state dot. */
export const DEVICE_STATE_TONE: Record<DeviceState, StatusDotTone & ('success' | 'warning' | 'muted')> = {
  online: 'success',
  updating: 'warning',
  offline: 'muted',
}
