import { t } from './locales/text'
import { inject, type InjectionKey } from 'vue'
import type { SentenceText } from '../ui/ui-text'

/**
 * How the device the page runs on unlocks, as the passkey offer names it
 * (`users.md` § User system). Told from the browser's system, as the
 * backend names a signed-in browser (`user_agent.rs`): the families, in the
 * order that tells them apart, never their versions.
 */
export type DeviceSystem = 'ios' | 'mac' | 'windows' | 'android' | 'other'

/** What a page reports of its device: the browser's User-Agent and how many touches its screen takes. */
export interface DeviceReport {
  userAgent: string
  maxTouchPoints: number
}

/**
 * The system the report names. An iPad's Safari reports a Mac; a Mac has no
 * touch screen, so a Mac with touches is an iPad.
 */
export function deviceSystem({ userAgent, maxTouchPoints }: DeviceReport): DeviceSystem {
  if (/iPhone|iPad/.test(userAgent)) {
    return 'ios'
  }
  if (/Android/.test(userAgent)) {
    return 'android'
  }
  if (/CrOS/.test(userAgent)) {
    return 'other'
  }
  if (/Windows/.test(userAgent)) {
    return 'windows'
  }
  if (/Macintosh/.test(userAgent)) {
    return maxTouchPoints > 1 ? 'ios' : 'mac'
  }
  return 'other'
}


/** What the passkey offer says a passkey does on this system. */
export function unlockSentence(system: DeviceSystem): SentenceText {
  return t(`unlock.${system}`)
}

/**
 * The device the offer speaks of. The page's own browser by default; a
 * catalog host provides another to show each system's wording.
 */
export const deviceReportKey: InjectionKey<DeviceReport> = Symbol('device-report')

/** The report the offer reads: a provided one, else this browser's. */
export function useDeviceReport(): DeviceReport {
  return inject(deviceReportKey, () => ({ userAgent: navigator.userAgent, maxTouchPoints: navigator.maxTouchPoints }), true)
}
