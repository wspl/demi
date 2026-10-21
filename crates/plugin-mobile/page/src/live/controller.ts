/**
 * A conversation's simulators on the page (`crates/plugin-mobile/page/src/index.ts`),
 * which the plugin's store keeps while the plugin is on (`./store.ts`): each
 * open device's live view, on a stream of its own, kept until the device
 * closes (`docs/decisions/0017-each-device-streams-on-its-own.md`), and the
 * panel's controls, each the page method of the command that does the same.
 */
import { ref, watch, type Ref } from 'vue'
import { clientPlatform } from '@demicodes/utils'
import { pageVisibility, picturesSupported, type HeadlineText, type PluginCache } from '@demicodes/plugin-sdk'
import { LIVE_NO_FOCUSED_FIELD, type PanelReason } from '../generated/plugin'
import { MobileError, type MobileApi } from '../devices'
import { DeviceSession } from './session'
import type { MobileTabData } from './kind'
import type { DeviceCall } from './controls'
import { t } from '../locales/text'

/**
 * A view notice's words: the reason's own, quoting the viewer's text the
 * notice carries where the reason is about it; never the Host's error text.
 */
export function noticeText(reason: PanelReason, message: string): string {
  return reason === LIVE_NO_FOCUSED_FIELD ? t(`reasons.${reason}`, { text: message }) : t(`reasons.${reason}`)
}

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
  /** The plugin's disk cache, which keeps the devices' frames; none keeps none. */
  cache?: PluginCache
}

export class MobileDevicesController {
  /** Whether the pictures can show: unknown until the decoder answered. */
  readonly pictures: Ref<'unknown' | 'supported' | 'unsupported'> = ref('unknown')
  /** Each open device's live view, by device, made as a view of it first shows. */
  private readonly devices = new Map<string, DeviceSession>()
  private readonly platform: 'mac' | 'windows' | 'linux' | 'other'
  private readonly cache: PluginCache | undefined
  /** Whether the page is visible. */
  private visible = true
  private readonly ticking: ReturnType<typeof setInterval>

  constructor(
    readonly api: MobileApi,
    private readonly errors: MobileErrors,
    options: MobileControllerOptions = {},
  ) {
    this.platform = options.platform ?? clientPlatform(navigator)
    this.cache = options.cache
    const supported = options.pictures ?? (() => picturesSupported((message, error) => errors.defect(message, error)))
    void supported().then((yes) => {
      this.pictures.value = yes ? 'supported' : 'unsupported'
      if (yes) {
        for (const session of this.devices.values()) {
          session.enable()
        }
      }
    })
    this.ticking = setInterval(() => {
      for (const session of this.devices.values()) {
        session.tick()
      }
    }, TICK_MS)
    // A hidden page looks at no device, so a device nobody looks at gets its
    // hour; the watch ends with the effect scope its store made it in.
    watch(pageVisibility, (visibility) => {
      this.visible = visibility !== 'hidden'
      for (const session of this.devices.values()) {
        session.visibility(this.visible)
      }
    }, { immediate: true })
  }

  /** The live view of the device `data` names, its tab's, made the first time. */
  device(data: MobileTabData): DeviceSession {
    const kept = this.devices.get(data.device)
    if (kept) {
      return kept
    }
    const session: DeviceSession = new DeviceSession({
      open: this.api.stream,
      device: data.device,
      model: data.model ?? null,
      nativeId: data.nativeId ?? null,
      cache: this.cache,
      platform: this.platform,
      onNotice: (reason: PanelReason, message: string) =>
        this.errors.report(t('failures.control'), new MobileError(reason, null, noticeText(reason, message))),
      // A device that closed takes its view along; its tab goes with it.
      gone: () => {
        if (this.devices.get(data.device) === session) {
          this.devices.delete(data.device)
        }
      },
      defect: (message, error) => this.errors.defect(message, error),
    })
    this.devices.set(data.device, session)
    session.visibility(this.visible)
    if (this.pictures.value === 'supported') {
      session.enable()
    }
    return session
  }

  /** The store released it, as when the plugin is turned off: every device's view ends and nothing reopens it. */
  dispose(): void {
    clearInterval(this.ticking)
    for (const session of this.devices.values()) {
      session.close()
    }
    this.devices.clear()
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
        case 'rotate': {
          // The shown picture turns at once, and back if the device refuses.
          const { device, turn } = call.params
          const session = this.devices.get(device)
          const press = session?.turning(turn) ?? null
          let answered
          try {
            answered = await this.api.rotate(device, turn)
          } catch (error) {
            session?.turnAnswered(press, null)
            throw error
          }
          session?.turnAnswered(press, answered.orientation)
          return null
        }
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
