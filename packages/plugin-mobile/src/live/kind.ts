import { defineComponent, h } from 'vue'
import { z } from 'zod'
import { Smartphone } from '@lucide/vue'
import { ICON_PX, type PanelKind } from '@demicodes/plugin-sdk'
import MobileTabContent from './MobileTabContent.vue'
import type { MobileDevicesController } from './controller'
import { t } from '../locales/text'

/**
 * What a `mobile` tab keeps: the device the agent opened, its native id
 * and its name, which the plugin writes as it opens the tab
 * (`mobile.md` § The live view).
 */
export const mobileTabDataSchema = z.looseObject({
  device: z.string(),
  nativeId: z.string().optional(),
  name: z.string(),
})
export type MobileTabData = z.infer<typeof mobileTabDataSchema>

const MobileTabMark = defineComponent({
  setup: () => () => h(Smartphone, { size: ICON_PX.markIn28 }),
})

/** The `mobile` tab kind: a device's live view. The panel opens no device of its own, so the kind has no New entry. */
export const mobileDeviceKind: PanelKind<MobileTabData, MobileDevicesController> = {
  kind: 'mobile',
  schema: mobileTabDataSchema,
  title: (data) => data.name,
  mark: MobileTabMark,
  content: MobileTabContent,
}
