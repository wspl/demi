// The `media` kind of the file-browser plugin's page: the Attachments window's one
// Media tab, which shows the medium the conversation's media viewer shows and
// steps through its series, as opening a picture or a video asks through the
// `media` intent.
import { defineComponent, h } from 'vue'
import { z } from 'zod'
import { Image, Images } from '@lucide/vue'
import { ICON_PX, type PanelKind, type PanelWindow } from '@demicodes/plugin-sdk'
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
  repeat: z.number().int().nonnegative().optional(),
})
export type MediaData = z.infer<typeof mediaDataSchema>

const MediaMark = defineComponent({
  setup: () => () => h(Image, { size: ICON_PX.markIn28 }),
})

/** Attachments' window: the conversation's pictures and videos, and each attachment opened. */
export const attachmentsWindow: PanelWindow = { id: 'attachments', name: () => t('window.attachments'), icon: Images, openable: false }

/**
 * The `media` kind: one tab, which every picture or video opened selects
 * again, showing it in place of the last.
 */
export const mediaKind: PanelKind<MediaData> = {
  kind: 'media',
  schema: mediaDataSchema,
  window: attachmentsWindow,
  title: () => t('kind.media'),
  key: () => 'media',
  mark: MediaMark,
  content: MediaTab,
  intents: {
    media: () => ({}),
  },
}
