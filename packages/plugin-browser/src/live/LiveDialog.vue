<script setup lang="ts">
import { nextTick, onMounted, ref, watch } from 'vue'
import type { LiveDialog } from '../generated/plugin'
import { Button } from '@demicodes/plugin-sdk'
import { ScrollArea } from '@demicodes/plugin-sdk'
import { TextInput } from '@demicodes/plugin-sdk'
import { dialogTitle, keyAnswer } from './dialogs'

/**
 * A dialog the watched page opened (`live-view.md` § A browser tab in the
 * panel). It shows over the picture without blocking the page: the agent
 * may answer it first, and then it goes away on its own. As in a browser, it
 * is titled with the page's host, takes the keyboard, a prompt in its
 * field, and answers Enter with its default button, OK or Leave, and Escape
 * with Cancel. The product's buttons take no focus of their own, so the
 * dialog holds it for them.
 */
const props = defineProps<{
  dialog: LiveDialog
  /** The host of the page that opened it; empty for a page without one, such as `about:blank`. */
  host: string
}>()
const emit = defineEmits<{ answer: [{ accept: boolean; text?: string }] }>()

const text = ref(props.dialog.defaultText)
const panel = ref<HTMLElement | null>(null)
const field = ref<InstanceType<typeof TextInput> | null>(null)

/** The dialog, or a prompt's field, takes the focus, so Enter and Escape reach the dialog. */
async function takeFocus(): Promise<void> {
  await nextTick()
  if (props.dialog.type === 'prompt') {
    field.value?.focus()
    field.value?.select()
    return
  }
  panel.value?.focus({ preventScroll: true })
}

onMounted(() => void takeFocus())
watch(() => props.dialog, (dialog) => {
  text.value = dialog.defaultText
  void takeFocus()
})

function answer(accept: boolean): void {
  emit('answer', props.dialog.type === 'prompt' && accept ? { accept, text: text.value } : { accept })
}

function key(event: KeyboardEvent): void {
  // The dialog's keys are the dialog's, never the page's under it.
  event.stopPropagation()
  const accept = event.isComposing ? null : keyAnswer(event.key, props.dialog.type)
  if (accept === null) {
    return
  }
  event.preventDefault()
  answer(accept)
}
</script>

<template>
  <div class="absolute inset-x-0 top-0 flex justify-center p-3">
    <div
      ref="panel"
      role="alertdialog"
      tabindex="-1"
      :aria-label="dialogTitle(dialog.type, host)"
      class="pointer-events-auto w-full max-w-md rounded-lg border border-line bg-surface p-3 shadow-lg outline-none"
      @keydown="key"
    >
      <p class="text-[12px] text-fg-subtle">{{ dialogTitle(dialog.type, host) }}</p>
      <ScrollArea class="mt-1 max-h-40">
        <p class="whitespace-pre-wrap break-words text-[13px] text-fg">
          {{ dialog.message }}
        </p>
      </ScrollArea>
      <TextInput
        v-if="dialog.type === 'prompt'"
        ref="field"
        v-model="text"
        class="mt-2 w-full"
        aria-label="Answer"
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
