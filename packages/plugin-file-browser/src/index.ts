// The `file-browser` plugin's page (`plugin-pages.md`, `file-previews.md`):
// the pinned `file` kind, the File view, which opens the `file` intent of
// the files messages name, and the `attachment` kind, which previews an
// attachment's copy for the `attachment` intent.
import { defineComponent, h, type PropType } from 'vue'
import { File } from '@lucide/vue'
import { FileIcon, ICON_PX, baseName, definePage, type PanelKind } from '@demicodes/plugin-sdk'
import { PLUGIN } from './generated/plugin'
import { t } from './locales/text'
import FileTab from './FileTab.vue'
import { fileDataSchema, firstFileData, openFile, type FileData } from './data'
import { attachmentKind } from './attachment'

/** The file's own icon once the view shows one, a plain file before. */
const FileMark = defineComponent({
  props: { data: { type: Object as PropType<FileData>, required: true } },
  setup: (props) => () => props.data.path
    ? h(FileIcon, { name: props.data.path, isDirectory: false, size: ICON_PX.markIn28 })
    : h(File, { size: ICON_PX.markIn28 }),
})

/** The `file` kind: the File view over the conversation's files service. */
export const fileKind: PanelKind<FileData> = {
  kind: 'file',
  schema: fileDataSchema,
  title: (data) => data.path ? t('kind.titleNamed', { name: baseName(data.path) }) : t('kind.title'),
  mark: FileMark,
  content: FileTab,
  pinned: { data: firstFileData },
  intents: {
    file: (payload, current) => openFile(current ?? firstFileData(), payload),
  },
}

export const fileBrowserPage = definePage({ plugin: PLUGIN, kinds: [fileKind, attachmentKind] })

/** The page the registry imports (`plugin-pages.md` § Registration). */
export default fileBrowserPage
