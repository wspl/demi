import { defineComponent, h, onBeforeUnmount, type PropType } from 'vue'
import { z } from 'zod'
import { ICON_PX, SystemLogo, usePage, type PanelKind } from '@demicodes/plugin-sdk'
import MobileTabContent from './MobileTabContent.vue'
import type { MobileStore } from './store'
import { mobilePlatformSchema, type MobilePlatform } from '../generated/plugin'
import { t } from '../locales/text'

/**
 * What a `mobile` tab keeps: the device the agent opened, its native id,
 * its name, its system and, when its driver knows it, its model, which the
 * plugin writes as it opens the tab (`crates/plugin-mobile/page/src/index.ts`).
 */
export const mobileTabDataSchema = z.looseObject({
  device: z.string(),
  nativeId: z.string().optional(),
  name: z.string(),
  platform: mobilePlatformSchema,
  model: z.string().optional(),
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

/**
 * The `mobile` tab kind over `store`: a device's live view, whose content
 * watches the conversation's simulators while it is mounted. The panel opens
 * no device of its own, so the kind has no New entry.
 */
export function mobileDeviceKind(store: MobileStore): PanelKind<MobileTabData> {
  const content = defineComponent({
    props: {
      conversation: { type: String, required: true },
      tabId: { type: String, required: true },
      data: { type: Object as PropType<MobileTabData>, required: true },
      shown: { type: Boolean, required: true },
    },
    setup(props) {
      // Watched while mounted: the conversation's simulators are the most recent to keep.
      const watched = usePage().store(store).watch(props.conversation)
      onBeforeUnmount(() => watched.end())
      return () => h(MobileTabContent, {
        conversation: props.conversation,
        session: watched.session.devices,
        tabId: props.tabId,
        data: props.data,
        shown: props.shown,
      })
    },
  })
  return {
    kind: 'mobile',
    schema: mobileTabDataSchema,
    title: (data) => data.name,
    mark: MobileTabMark,
    content,
  }
}
