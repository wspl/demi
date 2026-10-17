/**
 * A conversation's simulators in its panel (`mobile.md` § The live view):
 * one live view shared by the conversation's device tabs, which shows the
 * device of the tab shown, and the panel's controls, each the page method
 * of the command that does the same.
 */
import { ref, watch, type Ref } from 'vue'
import { clientPlatform } from '@demicodes/utils'
import { pageVisibility, picturesSupported, type HeadlineText } from '@demicodes/plugin-sdk'
import type { PanelReason } from '../generated/plugin'
import { MobileError, type MobileApi } from '../devices'
import { MobileSession } from './session'
import type { DeviceCall } from './controls'
import { t } from '../locales/text'

/** How often the view checks that its stream still speaks. */
const TICK_MS = 250

export interface MobileErrors {
  report(message: HeadlineText, error: unknown): void
  defect(message: string, error: unknown): void
}

export interface MobileControllerOptions {
  /** Whether this web browser shows the pictures; the page asks its decoder unless the gallery says. */
  pictures?: () => Promise<boolean>
  /** The stream's platform, the client's own unless the gallery says. */
  platform?: 'mac' | 'windows' | 'linux' | 'other'
}

export class MobileDevicesController {
  readonly session: MobileSession
  /** Whether the pictures can show: unknown until the decoder answered. */
  readonly pictures: Ref<'unknown' | 'supported' | 'unsupported'> = ref('unknown')
  private readonly ticking: ReturnType<typeof setInterval>

  constructor(
    readonly api: MobileApi,
    private readonly errors: MobileErrors,
    options: MobileControllerOptions = {},
  ) {
    this.session = new MobileSession({
      open: api.stream,
      platform: options.platform ?? clientPlatform(navigator),
      onNotice: (reason: PanelReason) => errors.report(t('failures.control'), new MobileError(reason, null, t(`reasons.${reason}`))),
      defect: (message, error) => errors.defect(message, error),
    })
    const supported = options.pictures ?? (() => picturesSupported((message, error) => errors.defect(message, error)))
    void supported().then((yes) => {
      this.pictures.value = yes ? 'supported' : 'unsupported'
      if (yes) {
        this.session.enable()
      }
    })
    this.ticking = setInterval(() => this.session.tick(), TICK_MS)
    // A hidden page watches no device, so a device nobody looks at gets its
    // hour; the watch ends with the panel session's scope.
    watch(pageVisibility, (visibility) => this.session.visibility(visibility !== 'hidden'), { immediate: true })
  }

  /** The panel closed: the view ends and nothing reopens it (`plugin-pages.md` § Panel sessions). */
  dispose(): void {
    clearInterval(this.ticking)
    this.session.close()
  }

  /**
   * Runs a toolbar control's call; answers the file on the Host that
   * Screenshot or a stopped Record left, which the content saves on the
   * user's computer. A refusal is a toast. Whether a device records is the
   * device's to say, in the view's state, whoever started it.
   */
  async run(call: DeviceCall): Promise<string | null> {
    try {
      switch (call.method) {
        case 'rotate':
          await this.api.rotate(call.params.device, call.params.turn)
          return null
        case 'toggleAppearance':
          await this.api.toggleAppearance(call.params.device)
          return null
        case 'press':
          await this.api.press(call.params.device, call.params.button)
          return null
        case 'screenshot':
          return (await this.api.screenshot(call.params.device)).path
        case 'record':
          return (await this.api.record(call.params.device, call.params.action)).path ?? null
      }
    } catch (error) {
      this.errors.report(t('failures.control'), error)
      return null
    }
  }
}
