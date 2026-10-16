<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { computed } from 'vue'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import type { OverlayStore } from '../overlay/overlayStore'
import { stopSentence, type MoveAnswers, type MoveQuestion } from './move-question'

/**
 * The question before a conversation that has messages moves
 * (`hosts.md` § Where a conversation runs): where it will run, that the
 * files stay where they are, and when the agent learns of it. Move and Tell
 * Agent is the default and moves it and wakes the agent; Move moves it only.
 * While the agent works the first line says what stops and the answers read
 * Stop and Move and Stop, Move and Tell Agent: they stop the work, and the
 * move follows once it is idle.
 * While the host carries the move out, the answer chosen shows that it
 * works and the other cannot be pressed (`answers`); Cancel closes the
 * dialog and the move goes on. `useMoveQuestion` holds its state. It stacks on a directory picker it is
 * asked from, which stays for another choice when the move is cancelled,
 * and replaces the host menu it is asked from.
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
  move: [tell: boolean]
}>()

/** The question as a sentence: while the agent works, moving stops it first. */
const title = computed(() => {
  const host = props.question?.host
  return props.question?.stops ? t('move.titleStop', { host }) : t('move.title', { host })
})
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
        <p v-if="question?.stops">{{ stopSentence(question.stops) }}</p>
        <p>
          <template v-if="question?.directory === '~'">{{ t('move.runsHome') }}</template>
          <template v-else-if="question?.directory">{{ runsIn[0] }}<span class="break-all text-fg-body">{{ question.directory }}</span>{{ runsIn[1] }}</template>
          <template v-else>{{ t('move.runsOwn') }}</template>
          {{ t('move.filesStay', { host: question?.fromCloud ? uiText('common.cloud') : (question?.from ?? '') }) }}
        </p>
        <p>{{ t('move.agentLearns') }}</p>
      </div>
    </div>
    <template #footer>
      <Button @click="emit('close')">{{ uiText('common.cancel') }}</Button>
      <Button :operation="answers?.move" @click="emit('move', false)">{{ question?.stops ? t('move.stopAndMove') : t('move.move') }}</Button>
      <Button
        variant="primary"
        :operation="answers?.tell"
        @click="emit('move', true)"
      >
        {{ question?.stops ? t('move.stopMoveTell') : t('move.moveAndTell') }}
      </Button>
    </template>
  </Dialog>
</template>
