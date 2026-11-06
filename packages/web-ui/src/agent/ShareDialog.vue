<script setup lang="ts">
import { computed } from 'vue'
import CopyLabel from '../ui/CopyLabel.vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import InlineError from '../ui/InlineError.vue'
import TextInput from '../ui/TextInput.vue'
import { shownError } from '../infra/errors'
import { t } from './locales/text'
import type { Action, Operation } from '../ui/operations'
import type { SharePhase, ShareTarget } from './sharing'

/**
 * The dialog of a share: who can read it, its link in a read-only field, and
 * the buttons Copy Link, Update Link (for a conversation's share) and Stop
 * Sharing, which stands apart at the leading edge since it ends the link. A
 * reply block's share is titled by the block's own title.
 *
 * The dialog opens before the link exists:
 *
 * - While `phase` makes the link, Creating link… stands in the field where
 *   the link will, with a spinner at the field's end, and Copy Link waits.
 * - A failure says why under the field, and Try Again takes Copy Link's place.
 * - Update Link shows its progress in the field the same way.
 *
 * Copy Link is the default button and turns to Copied after a copy, keeping its
 * width. Update Link
 * and Stop Sharing are the caller's `onUpdate` and `onStop`, which return the
 * promise of their work: the button shows it, and the other waits.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** What the share holds. */
  target: ShareTarget
  phase: SharePhase
  /** The link was just copied. */
  copied: boolean
  /** Update Link and Stop Sharing, each returning the promise of its work. */
  onUpdate?: Action
  onStop?: Action
}>()

const emit = defineEmits<{
  close: []
  copy: []
  retry: []
}>()

const title = computed(() => {
  switch (props.target.kind) {
    case 'conversation': return t('share.titleConversation')
    case 'answer': return t('share.titleAnswer')
    case 'reply_block': return t('share.titleReplyBlock', { title: props.target.title })
  }
})
const who = computed(() => {
  switch (props.target.kind) {
    case 'conversation': return t('share.whoConversation')
    case 'answer': return t('share.whoAnswer')
    case 'reply_block': return t('share.whoReplyBlock')
  }
})
const link = computed(() => (props.phase.status === 'ready' ? props.phase : null))
const failure = computed(() => (props.phase.status === 'failed' ? shownError(props.phase.error) : null))
const busy = computed(() => props.phase.status === 'creating' || props.phase.status === 'updating')
const token = computed(() => ('token' in props.phase ? props.phase.token : ''))
// Update and Stop never run together on one share.
const update = computed<Operation>(() => ({ action: 'update', target: `share:${token.value}`, group: 'share' }))
const stop = computed<Operation>(() => ({ action: 'stop', target: `share:${token.value}`, group: 'share' }))
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" :label="title" @close="emit('close')">
    <div class="flex flex-col gap-3 px-5 pt-5">
      <h3 class="select-none pr-8 text-[15px] font-medium text-fg-emphasis">{{ title }}</h3>
      <p class="text-explanation leading-5 text-fg-body">
        {{ who }}
      </p>
      <TextInput
        :model-value="link?.url ?? ''"
        :placeholder="busy ? (phase.status === 'creating' ? t('share.creating') : t('share.updating')) : undefined"
        :aria-label="t('share.field')"
        :aria-busy="busy || undefined"
        :invalid="phase.status === 'failed'"
        readonly
        literal
        autocomplete="off"
        @focus="($event.target as HTMLInputElement).select()"
      >
        <!-- At the field's end, so the text stands where the link will. -->
        <template v-if="busy" #suffix>
          <IndeterminateSpinner :size="14" />
        </template>
      </TextInput>
      <InlineError v-if="failure" :message="failure.message" :request-id="failure.requestId" />
    </div>
    <template #footer-leading>
      <Button variant="danger" :operation="stop" :disabled="!link" @click="onStop?.()">{{ t('share.stopSharing') }}</Button>
    </template>
    <template #footer>
      <Button v-if="target.kind === 'conversation'" :operation="update" :disabled="!link" @click="onUpdate?.()">{{ t('share.updateLink') }}</Button>
      <Button v-if="phase.status === 'failed'" variant="primary" @click="emit('retry')">{{ t('share.tryAgain') }}</Button>
      <Button v-else variant="primary" :disabled="!link" @click="emit('copy')">
        <CopyLabel :label="t('share.copyLink')" :copied="copied" />
      </Button>
    </template>
  </Dialog>
</template>
