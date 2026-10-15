<script setup lang="ts">
import { computed } from 'vue'
import { Check, Copy } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import TextInput from '../ui/TextInput.vue'
import { t as shared } from '../locales/text'
import { t } from './locales/text'
import type { Action, Operation } from '../ui/operations'

/**
 * The dialog of a share (`sharing.md` § Making, updating and removing a
 * share): who can read it, its link in a read-only field, and Copy Link,
 * Update Link for a conversation's share, and Stop Sharing, which stands
 * apart at the leading edge since it ends the link. Copy Link is the
 * default button and turns to Copied after a copy. Update Link and Stop
 * Sharing are the caller's `onUpdate` and `onStop`, which return the promise of
 * their work: the button shows it, and the other waits.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** What the share holds. */
  kind: 'conversation' | 'answer'
  url: string
  /** The link was just copied. */
  copied: boolean
  /** Which share's buttons these are, so their operations name it. */
  token: string
  /** Update Link and Stop Sharing, each returning the promise of its work. */
  onUpdate?: Action
  onStop?: Action
}>()

const emit = defineEmits<{
  close: []
  copy: []
}>()

const title = computed(() => (props.kind === 'conversation' ? t('share.titleConversation') : t('share.titleAnswer')))
// Update and Stop never run together on one share.
const update = computed<Operation>(() => ({ action: 'update', target: `share:${props.token}`, group: 'share' }))
const stop = computed<Operation>(() => ({ action: 'stop', target: `share:${props.token}`, group: 'share' }))
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" :label="title" @close="emit('close')">
    <div class="flex flex-col gap-3 px-5 pt-5">
      <h3 class="select-none pr-8 text-[15px] font-medium text-fg-emphasis">{{ title }}</h3>
      <p class="text-[13px] leading-5 text-fg-body">
        {{ kind === 'conversation' ? t('share.whoConversation') : t('share.whoAnswer') }}
      </p>
      <TextInput
        :model-value="url"
        :aria-label="t('share.field')"
        readonly
        literal
        autocomplete="off"
        @focus="($event.target as HTMLInputElement).select()"
      />
    </div>
    <template #footer-leading>
      <Button variant="danger" :operation="stop" @click="onStop?.()">{{ t('share.stopSharing') }}</Button>
    </template>
    <template #footer>
      <Button v-if="kind === 'conversation'" :operation="update" @click="onUpdate?.()">{{ t('share.updateLink') }}</Button>
      <Button variant="primary" @click="emit('copy')">
        <component :is="copied ? Check : Copy" :size="14" />
        {{ copied ? shared('common.copied') : t('share.copyLink') }}
      </Button>
    </template>
  </Dialog>
</template>
