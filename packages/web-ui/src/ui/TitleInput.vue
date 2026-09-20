<script setup lang="ts">
import { onMounted, ref } from 'vue'
import TextInput from './TextInput.vue'

/**
 * A title edited where it stands, in the framed field every text input is.
 * It arrives focused with the title selected and shown from its start, and
 * ends once: Enter or leaving it submits the trimmed title, Escape cancels.
 * An empty or unchanged title cancels too, so a caller only hears a real rename.
 */
const props = defineProps<{
  title: string
  /** Height family of the surface's other controls. */
  size?: 'sm' | 'md'
  label?: string
}>()

const emit = defineEmits<{
  submit: [title: string]
  cancel: []
}>()

const field = ref<InstanceType<typeof TextInput>>()
const value = ref(props.title)
// Enter ends the edit and the input then loses focus; the blur must not end it again.
let settled = false

function submit(): void {
  if (settled) {
    return
  }
  settled = true
  const title = value.value.trim()
  if (title && title !== props.title) {
    emit('submit', title)
  } else {
    emit('cancel')
  }
}

function cancel(): void {
  if (settled) {
    return
  }
  settled = true
  emit('cancel')
}

onMounted(() => {
  const input = field.value?.el
  if (!input) {
    return
  }
  input.focus()
  // Selected backward, the caret is at the start, and a title longer than the
  // field shows its beginning instead of its end.
  input.setSelectionRange(0, input.value.length, 'backward')
  input.scrollLeft = 0
})
</script>

<template>
  <TextInput
    ref="field"
    v-model="value"
    :size="size"
    :aria-label="label ?? 'Title'"
    @keydown.enter.stop.prevent="submit"
    @keydown.escape.stop.prevent="cancel"
    @keydown.stop
    @blur="submit"
    @click.stop
  />
</template>
