import { defineComponent, h, type PropType } from 'vue'
import { z } from 'zod'
import { ICON_PX, SystemLogo, type PanelKind } from '@demicodes/plugin-sdk'
import MobileTabContent from './MobileTabContent.vue'
import type { MobileDevicesController } from './controller'
import { mobilePlatformSchema, type MobilePlatform } from '../generated/plugin'
import { t } from '../locales/text'

/**
 * What a `mobile` tab keeps: the device the agent opened, its native id,
 * its name and its system, which the plugin writes as it opens the tab
 * (`packages/plugin-mobile/src/index.ts`).
 */
export const mobileTabDataSchema = z.looseObject({
  device: z.string(),
  nativeId: z.string().optional(),
  name: z.string(),
  platform: mobilePlatformSchema,
})
export type MobileTabData = z.infer<typeof mobileTabDataSchema>

/** A device's system's logo, as a device's mark draws it: Apple for iOS, the Android robot's head for Android. */
export const MobilePlatformLogo = defineComponent({
  props: { platform: { type: String as PropType<MobilePlatform>, required: true } },
  setup: (props) => () => h(SystemLogo, {
    logo: props.platform === 'ios' ? 'apple' : 'android',
    size: ICON_PX.markIn28,
  }),
})

const MobileTabMark = defineComponent({
  props: { data: { type: Object as PropType<MobileTabData>, required: true } },
  setup: (props) => () => h(MobilePlatformLogo, { platform: props.data.platform, class: 'text-fg-subtle' }),
})

/** The `mobile` tab kind: a device's live view. The panel opens no device of its own, so the kind has no New entry. */
export const mobileDeviceKind: PanelKind<MobileTabData, MobileDevicesController> = {
  kind: 'mobile',
  schema: mobileTabDataSchema,
  title: (data) => data.name,
  mark: MobileTabMark,
  content: MobileTabContent,
}
