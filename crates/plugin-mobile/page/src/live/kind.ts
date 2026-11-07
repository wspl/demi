import { computed, defineComponent, h, onBeforeUnmount, type PropType } from 'vue'
import { z } from 'zod'
import { Smartphone } from '@lucide/vue'
import { ICON_PX, SystemLogo, usePage, type PanelKind, type PanelWindow } from '@demicodes/plugin-sdk'
import MobileTabContent from './MobileTabContent.vue'
import OpenMenu from './OpenMenu.vue'
import OpeningStatus from './OpeningStatus.vue'
import type { MobileStore } from './store'
import { mobilePlatformSchema, type MobilePlatform } from '../generated/plugin'
import { t } from '../locales/text'

/** The kind's id, which the plugin's tabs name. */
export const KIND = 'mobile'

/**
 * What a `mobile` tab keeps: the device the conversation opened, its native id,
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

/** The simulators' window, a tab per simulator or emulator the conversation opened. */
const simulatorsWindow: PanelWindow = { id: 'simulators', name: () => t('window.name'), icon: Smartphone }

/**
 * The `mobile` tab kind over `store`: a device's live view, whose content
 * watches the conversation's simulators while it is mounted. A device opens
 * by the agent's command or the user's choice in the taskbar's Open
 * (`openMenu`), never as a blank tab, so the kind has no New entry.
 */
export function mobileDeviceKind(store: MobileStore): PanelKind<MobileTabData> {
  /** The conversation's simulators on the page, watched while the component is mounted. */
  function watching(conversation: string) {
    const watched = usePage().store(store).watch(conversation)
    onBeforeUnmount(() => watched.end())
    return watched.session.devices
  }
  const openMenu = defineComponent({
    props: { conversation: { type: String, required: true } },
    emits: { opened: () => true },
    setup(props, { emit }) {
      const controller = watching(props.conversation)
      return () => h(OpenMenu, { controller, onOpened: () => emit('opened') })
    },
  })
  const opening = defineComponent({
    props: { conversation: { type: String, required: true } },
    emits: { failed: () => true },
    setup(props, { emit }) {
      const page = usePage()
      const controller = watching(props.conversation)
      const shown = computed(() => new Set(page.panel.tabs(props.conversation, KIND).flatMap((data) => {
        const parsed = mobileTabDataSchema.safeParse(data)
        return parsed.success ? [parsed.data.device] : []
      })))
      return () => h(OpeningStatus, { controller, shown: shown.value, onFailed: () => emit('failed') })
    },
  })
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
    kind: KIND,
    schema: mobileTabDataSchema,
    window: simulatorsWindow,
    title: (data) => data.name,
    mark: MobileTabMark,
    content,
    openMenu,
    opening,
  }
}
