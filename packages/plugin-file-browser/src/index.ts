// The `file-browser` plugin's page (`plugin-pages.md`, `file-previews.md`):
// the pinned `file` kind, the File view, which opens the `file` intent of
// the files messages name.
import { defineComponent, h, type PropType } from 'vue'
import { File } from '@lucide/vue'
import {
  FileIcon,
  ICON_PX,
  baseName,
  type PanelKindsContext,
  type PanelTabKind,
  type PluginPage,
} from '@demicodes/plugin-sdk'
import FileTab from './FileTab.vue'
import { fileDataSchema, firstFileData, showFile, type FileData } from './data'

/** The file's own icon once the view shows one, a plain file before. */
const FileMark = defineComponent({
  props: { data: { type: Object as PropType<FileData>, required: true } },
  setup: (props) => () => props.data.path
    ? h(FileIcon, { name: props.data.path, isDirectory: false, size: ICON_PX.markIn28 })
    : h(File, { size: ICON_PX.markIn28 }),
})

/** The `file` kind of one conversation's panel, over its files service. */
function fileKind(context: PanelKindsContext): PanelTabKind<FileData> {
  const content = defineComponent({
    props: {
      tabId: { type: String, required: true },
      data: { type: Object as PropType<FileData>, required: true },
      shown: { type: Boolean, required: true },
    },
    emits: { update: (_data: FileData) => true },
    setup: (props, { emit }) => () =>
      h(FileTab, {
        data: props.data,
        files: context.files,
        onUpdate: (data: FileData) => emit('update', data),
      }),
  })
  return {
    kind: 'file',
    schema: fileDataSchema,
    title: (data) => data.path ? `File: ${baseName(data.path)}` : 'File',
    mark: FileMark,
    content,
    pinned: { data: firstFileData },
  }
}

/** What the tab shows now, or its first data while it has shown nothing. */
function current(data: unknown): FileData {
  const parsed = fileDataSchema.safeParse(data)
  return parsed.success ? parsed.data : firstFileData()
}

export const fileBrowserPage: PluginPage = {
  plugin: 'file-browser',
  panelKinds: (context) => ({ kinds: [fileKind(context)] }),
  intents: {
    file: { kind: 'file', open: (payload, data) => showFile(current(data), payload.path) },
  },
}

/** The page the registry imports (`plugin-pages.md` § Registration). */
export default fileBrowserPage
