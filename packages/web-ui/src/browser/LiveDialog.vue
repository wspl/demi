<script setup lang="ts">
import { ref, watch } from 'vue'
import type { LiveDialog } from '@demicodes/protocol'
import Button from '../ui/Button.vue'
import TextInput from '../ui/TextInput.vue'

/**
 * A dialog the watched page opened (`live-view.md` § Input). It shows
 * over the picture without blocking the page: the agent may answer it first,
 * and then it goes away on its own.
 */
const props = defineProps<{ dialog: LiveDialog }>()
const emit = defineEmits<{ answer: [{ accept: boolean; text?: string }] }>()

const text = ref(props.dialog.defaultText)
watch(() => props.dialog, (dialog) => {
  text.value = dialog.defaultText
})

const TITLES: Record<LiveDialog['type'], string> = {
  alert: 'The page says',
  confirm: 'The page asks',
  prompt: 'The page asks',
  beforeunload: 'Leave the page?',
}

function answer(accept: boolean): void {
  emit('answer', props.dialog.type === 'prompt' && accept ? { accept, text: text.value } : { accept })
}
</script>

<template>
  <div class="absolute inset-x-0 top-0 flex justify-center p-3">
    <div class="pointer-events-auto w-full max-w-md rounded-lg border border-line bg-surface p-3 shadow-lg">
      <p class="text-[12px] text-fg-subtle">{{ TITLES[dialog.type] }}</p>
      <p class="mt-1 max-h-40 overflow-auto whitespace-pre-wrap break-words text-[13px] text-fg">
        {{ dialog.message }}
      </p>
      <TextInput
        v-if="dialog.type === 'prompt'"
        v-model="text"
        class="mt-2 w-full"
        aria-label="Answer"
        @keydown.enter="answer(true)"
      />
      <div class="mt-3 flex justify-end gap-2">
        <Button
          v-if="dialog.type !== 'alert'"
          variant="default"
          size="sm"
          @click="answer(false)"
        >
          {{ dialog.type === 'beforeunload' ? 'Stay' : 'Cancel' }}
        </Button>
        <Button variant="primary" size="sm" @click="answer(true)">
          {{ dialog.type === 'beforeunload' ? 'Leave' : 'OK' }}
        </Button>
      </div>
    </div>
  </div>
</template>
