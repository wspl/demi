import { ref } from 'vue'
import { useEditor } from '@tiptap/vue-3'
import StarterKit from '@tiptap/starter-kit'
import Placeholder from '@tiptap/extension-placeholder'
import type { InputModel } from './input-model'

interface UseAgentInputEditorParams {
  initialValue?: InputModel | undefined
  handleSubmit: () => Promise<void> | void
  handleCancel: () => void
  handlePasteAttachments?: (clipboardData: DataTransfer, text: string) => boolean
}

export function useAgentInputEditor(params: UseAgentInputEditorParams) {
  const isFocused = ref(false)

  const editor = useEditor({
    content: params.initialValue ?? null,
    extensions: [
      StarterKit.configure({
        bold: false,
        italic: false,
        heading: false,
        bulletList: false,
        orderedList: false,
        blockquote: false,
        codeBlock: false,
        code: false,
        horizontalRule: false,
        listItem: false,
        strike: false,
      }),
      Placeholder.configure({ placeholder: 'Send a message...' }),
    ],
    editorProps: {
      attributes: {
        class: 'w-full resize-none bg-transparent px-4 pt-3 pb-2 text-[13px] leading-relaxed text-fg outline-none max-h-40 overflow-y-auto',
      },
      handleKeyDown(_view, event) {
        if (event.isComposing) return false

        if (event.key === 'Escape') {
          params.handleCancel()
          return true
        }

        const shouldSubmit = event.key === 'Enter' && !event.shiftKey
        if (shouldSubmit) {
          event.preventDefault()
          void params.handleSubmit()
          return true
        }

        return false
      },
      handlePaste(_view, event) {
        const clipboardData = event.clipboardData
        if (!clipboardData || !params.handlePasteAttachments) return false
        const text = clipboardData.getData('text/plain')
        if (params.handlePasteAttachments(clipboardData, text)) {
          event.preventDefault()
          return true
        }
        return false
      },
    },
    onFocus() {
      isFocused.value = true
    },
    onBlur() {
      isFocused.value = false
    },
  })

  return { editor, isFocused }
}
