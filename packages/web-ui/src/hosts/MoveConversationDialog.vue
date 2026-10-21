<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { computed } from 'vue'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { MoveAnswers, MoveQuestion } from './move-question'

/**
 * The question before a conversation that has messages moves: where it will run,
 * and that the files stay where they are.
 *
 * - Move, the default, moves it at once, whether or not its agent works: nothing
 *   stops, and each agent's next step runs in the new place.
 * - While the host carries the move out, Move shows that it works and cannot be
 *   pressed again (`answers`). Cancel closes the dialog and the move goes on.
 * - `useMoveQuestion` holds its state.
 * - It stacks on a directory picker it is asked from, which stays for another
 *   choice when the move is cancelled, and replaces the host menu it is asked
 *   from.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  question: MoveQuestion | null
  /** What each answer does (`useMoveQuestion`). */
  answers?: MoveAnswers
}>()

const emit = defineEmits<{
  close: []
  move: []
}>()

const title = computed(() => t('move.title', { host: props.question?.host }))
// The directory stands in its own span, so the sentence is split around where it goes.
const DIRECTORY_MARK = '\u0001'
const runsIn = computed(() => t('move.runsIn', { directory: DIRECTORY_MARK }).split(DIRECTORY_MARK))
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :label="title"
    stack
    @close="emit('close')"
  >
    <div class="flex flex-col gap-4 px-5 pt-5">
      <h3 class="pr-8 text-[15px] font-medium text-fg-emphasis">
        {{ title }}
      </h3>
      <div class="flex flex-col gap-3 text-explanation leading-5 text-fg-muted">
        <p>
          <template v-if="question?.directory === '~'">{{ t('move.runsHome') }}</template>
          <template v-else-if="question?.directory">{{ runsIn[0] }}<span class="break-all text-fg-body">{{ question.directory }}</span>{{ runsIn[1] }}</template>
          <template v-else>{{ t('move.runsOwn') }}</template>
          {{ t('move.filesStay', { host: question?.fromCloud ? uiText('common.cloud') : (question?.from ?? '') }) }}
        </p>
      </div>
    </div>
    <template #footer>
      <Button @click="emit('close')">{{ uiText('common.cancel') }}</Button>
      <Button variant="primary" :operation="answers?.move" @click="emit('move')">{{ t('move.move') }}</Button>
    </template>
  </Dialog>
</template>
