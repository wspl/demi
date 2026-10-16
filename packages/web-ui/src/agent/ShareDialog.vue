<script setup lang="ts">
import { computed } from 'vue'
import { Check, Copy } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import InlineError from '../ui/InlineError.vue'
import TextInput from '../ui/TextInput.vue'
import { shownError } from '../infra/errors'
import { t as shared } from '../locales/text'
import { t } from './locales/text'
import type { Action, Operation } from '../ui/operations'
import type { SharePhase } from './sharing'

/**
 * The dialog of a share (`sharing.md` § Making, updating and removing a
 * share): who can read it, its link in a read-only field, and Copy Link,
 * Update Link for a conversation's share, and Stop Sharing, which stands
 * apart at the leading edge since it ends the link. It opens before the
 * link exists: while `phase` makes it, Creating link… and a spinner stand
 * in the field where the link will, and Copy Link waits; a failure says why
 * under the field, and Try Again takes Copy Link's place; Update Link shows
 * its progress in the field the same way. Copy Link is the
 * default button and turns to Copied after a copy. Update Link and Stop
 * Sharing are the caller's `onUpdate` and `onStop`, which return the promise
 * of their work: the button shows it, and the other waits.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** What the share holds. */
  kind: 'conversation' | 'answer'
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

const title = computed(() => (props.kind === 'conversation' ? t('share.titleConversation') : t('share.titleAnswer')))
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
        {{ kind === 'conversation' ? t('share.whoConversation') : t('share.whoAnswer') }}
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
        <template v-if="busy" #prefix>
          <IndeterminateSpinner :size="14" />
        </template>
      </TextInput>
      <InlineError v-if="failure" :message="failure.message" :request-id="failure.requestId" />
    </div>
    <template #footer-leading>
      <Button variant="danger" :operation="stop" :disabled="!link" @click="onStop?.()">{{ t('share.stopSharing') }}</Button>
    </template>
    <template #footer>
      <Button v-if="kind === 'conversation'" :operation="update" :disabled="!link" @click="onUpdate?.()">{{ t('share.updateLink') }}</Button>
      <Button v-if="phase.status === 'failed'" variant="primary" @click="emit('retry')">{{ t('share.tryAgain') }}</Button>
      <Button v-else variant="primary" :disabled="!link" @click="emit('copy')">
        <component :is="copied ? Check : Copy" :size="14" />
        {{ copied ? shared('common.copied') : t('share.copyLink') }}
      </Button>
    </template>
  </Dialog>
</template>
