/**
 * The panel's controls and the `mobile` user stream over the plugin
 * (`packages/plugin-mobile/src/index.ts`): each control is a page method that runs
 * the operation the agent's command of the same meaning runs. A call still
 * waiting when the panel session ends is dropped and never sent
 * (`packages/web-ui/src/plugins/page.ts`). The panel opens no device: the
 * agent does, and the plugin gives each device its tab.
 */
import { onScopeDispose } from 'vue'
import type { z } from 'zod'
import { PluginCallError, type ConversationPlugin, type OpenUserStream } from '@demicodes/plugin-sdk'
import {
  appearanceResultSchema,
  closeResultSchema,
  keyResultSchema,
  panelReasonSchema,
  recordResultSchema,
  rotateResultSchema,
  savedSchema,
  type OnDevice,
  type PanelButton,
  type PanelReason,
  type PressButton,
  type RecordAction,
  type RecordDevice,
  type RecordResult,
  type RotateDevice,
  type Saved,
  type Turn,
} from './generated/plugin'
import { t } from './locales/text'

/** A refused call, in the words the page shows for it, never the Host's own error text. */
export class MobileError extends Error {
  constructor(readonly reason: PanelReason | null, readonly status: string | null, message: string) {
    super(message)
  }

  static of(error: unknown): MobileError {
    if (error instanceof MobileError) {
      return error
    }
    if (error instanceof PluginCallError) {
      const reason = panelReasonSchema.safeParse(error.reason)
      if (reason.success) {
        return new MobileError(reason.data, null, t(`reasons.${reason.data}`))
      }
      const status = error.answer.type === 'status' ? String(error.answer.status) : error.answer.type
      return new MobileError(null, status, t('status', { status }))
    }
    return new MobileError(null, null, error instanceof Error ? error.message : String(error))
  }
}

export interface MobileApi {
  close(device: string): Promise<void>
  rotate(device: string, turn: Turn): Promise<void>
  toggleAppearance(device: string): Promise<void>
  press(device: string, button: PanelButton): Promise<void>
  screenshot(device: string): Promise<Saved>
  record(device: string, action: RecordAction): Promise<RecordResult>
  stream: OpenUserStream
}

export function mobileApi(plugin: ConversationPlugin): MobileApi {
  const session = new AbortController()
  onScopeDispose(() => session.abort())
  async function call<T>(method: string, params: object, answer: z.ZodType<T>): Promise<T> {
    try {
      return await plugin.call(method, params, answer, { signal: session.signal })
    } catch (error) {
      if (session.signal.aborted) {
        throw error
      }
      throw MobileError.of(error)
    }
  }
  return {
    close: async (device) => {
      await call('close', { device } satisfies OnDevice, closeResultSchema)
    },
    rotate: async (device, turn) => {
      await call('rotate', { device, turn } satisfies RotateDevice, rotateResultSchema)
    },
    toggleAppearance: async (device) => {
      await call('toggleAppearance', { device } satisfies OnDevice, appearanceResultSchema)
    },
    press: async (device, button) => {
      await call('press', { device, button } satisfies PressButton, keyResultSchema)
    },
    screenshot: (device) => call('screenshot', { device } satisfies OnDevice, savedSchema),
    record: (device, action) => call('record', { device, action } satisfies RecordDevice, recordResultSchema),
    stream: plugin.stream('mobile'),
  }
}
