<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import Button from '../../ui/Button.vue'
import TextArea from '../../ui/TextArea.vue'
import { t } from '../locales/text'
import { t as shared } from '../../locales/text'

/**
 * A waiting comment being edited: its text in a field, with Save and
 * Cancel. Return saves, Shift-Return starts a new line, Escape cancels.
 */
const props = defineProps<{
  text: string
}>()

const emit = defineEmits<{
  save: [text: string]
  cancel: []
}>()

const draft = ref(props.text)
const root = ref<HTMLElement>()
const empty = computed(() => draft.value.trim() === '')

// Edit asked for the field: it takes the focus, on a phone too, the caret at
// the end.
onMounted(() => {
  const field = root.value?.querySelector('textarea')
  field?.focus({ preventScroll: true })
  field?.setSelectionRange(draft.value.length, draft.value.length)
})

function save(): void {
  if (!empty.value)
    emit('save', draft.value.trim())
}

function onKeydown(event: KeyboardEvent): void {
  if (event.isComposing)
    return
  if (event.key === 'Enter' && !event.shiftKey) {
    event.preventDefault()
    save()
  } else if (event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    emit('cancel')
  }
}
</script>

<template>
  <div ref="root" class="flex flex-col gap-2" @keydown="onKeydown">
    <TextArea v-model="draft" :rows="2" />
    <div class="flex items-center justify-end gap-2">
      <Button size="sm" @click="emit('cancel')">{{ shared('common.cancel') }}</Button>
      <Button size="sm" variant="primary" :disabled="empty" :disabled-reason="t('comments.writeFirst')" @click="save">{{ shared('common.save') }}</Button>
    </div>
  </div>
</template>
