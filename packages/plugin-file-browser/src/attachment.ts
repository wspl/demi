// The `attachment` kind of the file-browser plugin's page: an attachment of
// the conversation previewed from its stored copy, as its block's card opens
// it through the `attachment` intent (`file-previews.md` § Attachment blocks).
import { defineComponent, h, type PropType } from 'vue'
import { z } from 'zod'
import { FileIcon, ICON_PX, type IntentPayloads, type PanelKind } from '@demicodes/plugin-sdk'
import AttachmentTab from './AttachmentTab.vue'

/** The attachment a tab shows: its number, its file's name and size, and where its copy loads from. */
export const attachmentDataSchema = z.object({
  id: z.string(),
  name: z.string(),
  size: z.number().int().nonnegative(),
  url: z.string(),
})
export type AttachmentData = z.infer<typeof attachmentDataSchema>

/** The tab's data for the `attachment` intent: the attachment it names, as it names it. */
export function openAttachment(payload: IntentPayloads['attachment']): AttachmentData {
  return { id: payload.id, name: payload.name, size: payload.size, url: payload.url }
}

/** The file's own icon. */
const AttachmentMark = defineComponent({
  props: { data: { type: Object as PropType<AttachmentData>, required: true } },
  setup: (props) => () => h(FileIcon, { name: props.data.name, isDirectory: false, size: ICON_PX.markIn28 }),
})

/**
 * The `attachment` kind: each attachment opened gets a tab of its own named
 * by its file, as a browser opens a document in a tab; a second click on its
 * card selects its tab again, whatever the panel shows.
 */
export const attachmentKind: PanelKind<AttachmentData> = {
  kind: 'attachment',
  schema: attachmentDataSchema,
  title: (data) => data.name,
  key: (data) => data.id,
  mark: AttachmentMark,
  content: AttachmentTab,
  intents: {
    attachment: (payload) => openAttachment(payload),
  },
}
