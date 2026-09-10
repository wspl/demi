<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { X } from '@lucide/vue'
import Dialog from '../ui/Dialog.vue'
import Button from '../ui/Button.vue'
import IconButton from '../ui/IconButton.vue'
import InlineError from '../ui/InlineError.vue'
import ContentMedia from './ContentMedia.vue'
import AttachmentTile from './AttachmentTile.vue'
import type { OverlayStore } from '../overlay/overlayStore'
import { decodeRemoteReference } from './message-input/attachments'
import {
  changeMessageEditContent,
  editHasContent,
  isMessageEditSubmitKey,
  type MessageEditState,
} from './message-editing'

const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  state: MessageEditState
}>()
const emit = defineEmits<{
  close: []
  cancel: []
  submit: []
  update: [state: MessageEditState]
}>()
const editable = computed(() => props.state.phase === 'editing')
const busy = computed(() => props.state.phase === 'sending')
const form = ref<HTMLFormElement | null>(null)
watch(() => props.isOpen, async (open) => {
  if (open) {
    await nextTick()
    if (props.isOpen) {
      form.value?.querySelector('textarea')?.focus()
    }
  }
}, { immediate: true })

function changeText(index: number, event: Event): void {
  if (!editable.value) {
    return
  }
  emit('update', changeMessageEditContent(props.state, (content) => {
    const part = content[index]
    if (part?.type === 'text') {
      part.text = (event.target as HTMLTextAreaElement).value
    }
  }))
}

function removeAttachment(index: number): void {
  if (!editable.value) {
    return
  }
  emit('update', changeMessageEditContent(props.state, (content) => {
    content.splice(index, 1)
  }))
}

function submit(): void {
  if (!busy.value && editHasContent(props.state)) {
    emit('submit')
  }
}

function keydown(event: KeyboardEvent): void {
  if (isMessageEditSubmitKey(event)) {
    event.preventDefault()
    submit()
  }
}
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    label="Edit message"
    size="wide"
    @close="emit('close')"
  >
    <form ref="form" class="flex flex-col gap-4 p-5" @submit.prevent="submit">
      <header class="pr-10">
        <h2 class="text-chrome font-medium text-fg-emphasis">Edit message</h2>
        <p class="mt-1 text-chrome leading-relaxed text-fg-muted">
          Saving replaces this message and all later messages, then generates a new reply.
        </p>
      </header>
      <div class="flex flex-col gap-3">
        <template v-for="(part, index) in state.request.content" :key="index">
          <textarea
            v-if="part.type === 'text'"
            :value="part.text"
            :disabled="!editable"
            :aria-label="`Message text ${index + 1}`"
            rows="5"
            class="min-h-32 w-full resize-y rounded-lg border border-line bg-surface px-3 py-2 text-conversation text-fg outline-none focus:border-fg-faint disabled:opacity-70"
            @input="changeText(index, $event)"
            @keydown="keydown"
          />
          <div v-else class="flex items-start gap-2">
            <AttachmentTile
              v-if="part.type === 'reference'"
              :name="decodeRemoteReference(part.reference).name"
            />
            <ContentMedia
              v-else
              :kind="part.type"
              :source="part.source"
              :name="'fileName' in part.source ? part.source.fileName ?? part.type : part.type"
            />
            <IconButton
              v-if="editable"
              :icon="X"
              variant="ghost"
              aria-label="Remove attachment"
              @click="removeAttachment(index)"
            />
          </div>
        </template>
      </div>
      <InlineError v-if="state.error" :message="state.error" />
      <p v-if="state.phase === 'uncertain'" class="text-chrome text-fg-muted">
        Retry checks whether this edit was accepted.
      </p>
      <footer class="flex items-center justify-end gap-2">
        <Button v-if="editable" variant="ghost" @click="emit('cancel')">Cancel</Button>
        <Button v-else variant="ghost" @click="emit('close')">Close</Button>
        <Button :loading="busy" :disabled="!editHasContent(state)" @click="submit">
          {{ state.phase === 'uncertain' ? 'Retry' : 'Save and resend' }}
        </Button>
      </footer>
    </form>
  </Dialog>
</template>
