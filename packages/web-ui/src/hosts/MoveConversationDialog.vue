<script setup lang="ts">
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { MoveQuestion } from './move-question'

/**
 * The question before a conversation that has messages moves
 * (`product.md` § Where a conversation runs): where it will run, that the
 * files stay where they are, and when the agent learns of it. Move and Tell
 * Agent is the default and moves it and wakes the agent; Move moves it only.
 * While the host carries the move out, `busy` keeps the dialog open;
 * `useMoveQuestion` holds its state. It stacks on a directory picker it is
 * asked from, which stays for another choice when the move is cancelled,
 * and replaces the host menu it is asked from.
 */
defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  question: MoveQuestion | null
  /** Which answer is being carried out. */
  busy?: 'move' | 'tell' | null
}>()

const emit = defineEmits<{
  close: []
  move: [tell: boolean]
}>()
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :label="`Move this conversation to ${question?.host}?`"
    stack
    @close="emit('close')"
  >
    <div class="flex flex-col gap-4 px-5 pt-5">
      <h3 class="pr-8 text-[15px] font-medium text-fg-emphasis">
        Move this conversation to {{ question?.host }}?
      </h3>
      <div class="flex flex-col gap-3 text-[13px] leading-5 text-fg-muted">
        <p>
          <template v-if="question?.directory === '~'">It will run in the home directory there.</template>
          <template v-else-if="question?.directory">It will run in <span class="break-all text-fg-body">{{ question.directory }}</span>.</template>
          <template v-else>It will run in a directory of its own there.</template>
          Files on {{ question?.fromCloud ? 'the Cloud' : question?.from }} stay there.
        </p>
        <p>The agent learns of the move when it next works, or now if you tell it.</p>
      </div>
    </div>
    <template #footer>
      <Button :disabled="!!busy" @click="emit('close')">Cancel</Button>
      <Button :disabled="!!busy" :loading="busy === 'move'" @click="emit('move', false)">Move</Button>
      <Button
        variant="primary"
        :disabled="!!busy"
        :loading="busy === 'tell'"
        @click="emit('move', true)"
      >
        Move and Tell Agent
      </Button>
    </template>
  </Dialog>
</template>
