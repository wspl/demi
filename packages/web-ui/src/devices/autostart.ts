import { t } from './locales/text'
import { homePath } from '../files/paths'
import type { SentenceText } from '../ui/ui-text'

/**
 * Why a device will not start Demi by itself after a restart, as its runner
 * last reported it (`runner.md` § Starting with the computer): its cause,
 * with what the system said.
 */
export type AutostartUnavailable =
  | { cause: 'noSystemd' }
  | { cause: 'noUserManager'; error: string }
  | { cause: 'notWritten'; path: string; error: string }
  | { cause: 'lingerOff'; user: string }
  | { cause: 'refused'; error: string }
  | { cause: 'unusableName'; name: string }
  | { cause: 'unknownAccount' }

/**
 * The cause as a person reads it: a plain sentence first, a path from `~`
 * under the device's `home`, what the system said after it; and the command
 * that removes the cause, when one does.
 */
export function autostartCause(
  reason: AutostartUnavailable,
  home: string | null | undefined,
): { sentence: SentenceText; command: string | null } {
  switch (reason.cause) {
    case 'noSystemd':
      return { sentence: t('autostart.noSystemd'), command: null }
    case 'noUserManager':
      return { sentence: t('autostart.noUserManager', { error: reason.error }), command: null }
    case 'notWritten':
      return {
        sentence: t('autostart.notWritten', { path: homePath(reason.path, home), error: reason.error }),
        command: null,
      }
    case 'lingerOff':
      return {
        sentence: t('autostart.lingerOff', { user: reason.user }),
        command: `sudo loginctl enable-linger ${reason.user}`,
      }
    case 'refused':
      return { sentence: t('autostart.refused', { error: reason.error }), command: null }
    case 'unusableName':
      return { sentence: t('autostart.unusableName', { name: reason.name }), command: null }
    case 'unknownAccount':
      return { sentence: t('autostart.unknownAccount'), command: null }
  }
}
