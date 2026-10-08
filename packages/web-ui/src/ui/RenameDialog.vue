<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from './Button.vue'
import Dialog from './Dialog.vue'
import TextInput from './TextInput.vue'
import { useAutofocus } from './autofocus'
import type { HeadlineText, SentenceText } from './ui-text'

/**
 * A new name, asked for as macOS asks in a sheet: the field opens on the
 * current name, selected, so typing replaces it. Rename, or Return in the
 * field, hands over the name trimmed and closes; Cancel or Escape keeps the
 * old one. An empty name cannot be taken, and the field holds at most
 * `maxLength` characters. An unchanged name closes without a rename. The
 * host says a refusal in a toast.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The command's name without its ellipsis: Rename Device. */
  title: HeadlineText
  /** The field's accessible name: Device name. */
  label: SentenceText
  /** The current name, which the field opens on. */
  name: string
  /** The most characters the name has; null for no limit. */
  maxLength: number | null
}>()
const emit = defineEmits<{
  close: []
  rename: [name: string]
}>()

const draft = ref(props.name)
watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      draft.value = props.name
    }
  },
)
const trimmed = computed(() => draft.value.trim())

const field = ref<InstanceType<typeof TextInput>>()
const autofocus = useAutofocus()
// The field mounts with each opening; it takes the focus with the name
// selected. Where it may not take the focus (a catalog host), nothing is
// selected either.
watch(
  field,
  (input) => {
    const element = input?.el
    if (!element) {
      return
    }
    autofocus(element)
    if (document.activeElement === element) {
      element.select()
    }
  },
  { flush: 'post' },
)

function submit() {
  if (!trimmed.value) {
    return
  }
  if (trimmed.value !== props.name) {
    emit('rename', trimmed.value)
  }
  emit('close')
}
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :label="title"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-4 p-5">
      <h3 class="select-none pr-8 text-[15px] font-medium text-fg-emphasis">{{ title }}</h3>
      <TextInput
        ref="field"
        v-model="draft"
        :aria-label="label"
        :maxlength="maxLength ?? undefined"
        literal
        autocomplete="off"
        @keydown.enter="submit"
      />
      <div class="flex justify-end gap-2">
        <Button @click="emit('close')">Cancel</Button>
        <Button variant="primary" :disabled="!trimmed" @click="submit">Rename</Button>
      </div>
    </div>
  </Dialog>
</template>
