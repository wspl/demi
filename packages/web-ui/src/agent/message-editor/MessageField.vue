<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { showToast } from '../../infra/toast'
import type { PlaceholderText } from '../../ui/ui-text'
import type { DraftMessage, MessageFiles } from '../comments/comments'
import { missingFilesToast, pasteMessage, type CopiedMessage } from '../message-input/copied-message'
import { provideTransfers, type MessageCapsule } from './capsules'
import MessageEditor from './MessageEditor.vue'

/**
 * A message written beside the composer's, as a comment is: the composer's
 * editor, its files taken, uploaded and pasted as the composer takes them,
 * among the conversation's (`MessageFiles`): pasted or dropped. Its owner
 * draws what goes around it.
 *
 * The files it holds stay while it is open, whatever else lets files go;
 * they go with it when it closes, unless its message was saved.
 */
const props = defineProps<{
  files: MessageFiles
  placeholder?: PlaceholderText
  /** What assistive technology calls the field. */
  label?: string
  /** Takes the focus once it shows, the caret at the end. */
  autofocus?: boolean
  /** Escape ends something, such as the field. */
  cancelable?: boolean
  /** How wide one line of the field is, as the composer measures its own. */
  lineWidth?: number
  /** The document the editor is made in, a frame's (`FramedField.vue`). */
  document?: Document
}>()

const message = defineModel<DraftMessage>({ required: true })
/** The message needs more than one line. */
const multiline = defineModel<boolean>('multiline', { default: false })

const emit = defineEmits<{
  submit: [otherWay: boolean]
  cancel: []
  focus: []
  blur: []
}>()

/** Names this field among the editors that hold files. */
const editorId = crypto.randomUUID()
const editor = ref<InstanceType<typeof MessageEditor>>()

/**
 * Every file this field's message held, which an undo can bring back: a
 * capsule pasted from this field keeps its file, and one from another
 * message's editor comes in as a copy, with a file of its own.
 */
const took = new Set<string>()
provideTransfers({ ...props.files.transfers, carries: (id) => took.has(id) })

/** What the editor shows: the message's capsules, each of a file the conversation holds. */
const attachments = computed(() => props.files.capsules(message.value.fileIds))

watch(() => message.value.fileIds, (ids) => {
  ids.forEach((id) => took.add(id))
  props.files.hold(editorId, ids)
}, { immediate: true })
onBeforeUnmount(() => props.files.hold(editorId, null))

function change(markdown: string, capsules: MessageCapsule[]): void {
  message.value = { text: markdown, fileIds: capsules.map((capsule) => capsule.id) }
}

/** Files picked, pasted or dropped: taken, and their capsules put where they landed. */
function addFiles(files: File[]): void {
  editor.value?.insertCapsules(props.files.add(files))
}

/** A copied message pasted: its text and capsules where it was pasted, its stored files attaching. */
function pasteCopied(copied: CopiedMessage, image: File | null): void {
  const target = editor.value
  const pasted = pasteMessage(copied, image, props.files.paste)
  target?.insertMessage(pasted.markdown, pasted.capsules)
  void Promise.all(pasted.attaching.map(async ({ id, name, kept }) =>
    !await kept && target?.replaceCapsule(id, name) ? [name] : []))
    .then((names) => {
      const toast = missingFilesToast([...pasted.missing, ...names.flat()])
      if (toast)
        showToast({ ...toast, tone: 'danger' })
    })
}

defineExpose({
  /** Takes the focus, the caret where it was. */
  focus(): void {
    editor.value?.focus()
  },
  /** Takes the focus as the page gives it, the caret at the end. */
  takeFocus(): void {
    editor.value?.takeFocus()
  },
})
</script>

<template>
  <div class="min-w-0">
    <MessageEditor
      ref="editor"
      v-model:multiline="multiline"
      :markdown="message.text"
      :attachments="attachments"
      :placeholder="placeholder"
      :label="label"
      :autofocus="autofocus"
      :asked="autofocus"
      :cancelable="cancelable"
      :line-width="lineWidth"
      :document="document"
      @change="change"
      @submit="emit('submit', $event)"
      @cancel="emit('cancel')"
      @files="addFiles"
      @message="pasteCopied"
      @focus="emit('focus')"
      @blur="emit('blur')"
    />
  </div>
</template>
