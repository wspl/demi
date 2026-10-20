// The `media` kind of the file-browser plugin's page: the work panel's one
// Media tab, which shows the medium the conversation's media viewer shows and
// steps through its series, as opening a picture or a video asks through the
// `media` intent.
import { defineComponent, h } from 'vue'
import { z } from 'zod'
import { Image } from '@lucide/vue'
import { ICON_PX, type PanelKind } from '@demicodes/plugin-sdk'
import { t } from './locales/text'
import MediaTab from './MediaTab.vue'

/**
 * The tab's data: where the medium it shows stands in the conversation's
 * series (`MediumPlace`), which it shows again after a reload; the viewer
 * holds the medium itself. Empty before it shows one.
 */
export const mediaDataSchema = z.object({
  key: z.string().optional(),
  row: z.string().optional(),
})
export type MediaData = z.infer<typeof mediaDataSchema>

const MediaMark = defineComponent({
  setup: () => () => h(Image, { size: ICON_PX.markIn28 }),
})

/**
 * The `media` kind: one tab, which every picture or video opened selects
 * again, showing it in place of the last.
 */
export const mediaKind: PanelKind<MediaData> = {
  kind: 'media',
  schema: mediaDataSchema,
  title: () => t('kind.media'),
  key: () => 'media',
  mark: MediaMark,
  content: MediaTab,
  intents: {
    media: () => ({}),
  },
}
