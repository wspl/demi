<script setup lang="ts">
import { useClipboard } from '@vueuse/core'
import { Check, CircleX, Copy } from '@lucide/vue'
import { t } from '../infra/i18n'
import Button from './Button.vue'
import IconButton from './IconButton.vue'
import Tooltip from './Tooltip.vue'

/**
 * A failure told in the flow of the conversation: a turn that failed, a
 * message that was not delivered, a session that lost its connection.
 *
 * One tinted bar, full width where it sits: a sentence, the upstream message
 * under it, the diagnostic facts on one line, and at most one action beside a
 * copy control for the support thread. It is a record in the transcript, not a
 * fixture of the dock, so it scrolls with what it describes.
 */
const props = withDefaults(
  defineProps<{
    label: string
    /** The upstream message or reason, in the caller's words. */
    detail?: string | null
    /** The short facts a support thread asks for first: status, code, request id. */
    facts?: readonly string[]
    /** The single action's label; the caller decides what it does. */
    action?: string
    /** What the copy control puts on the clipboard; no control without it. */
    copyText?: string | null
  }>(),
  {
    detail: null,
    facts: () => [],
    copyText: null,
  },
)
defineEmits<{ action: [] }>()
const { copy, copied } = useClipboard({ copiedDuring: 1500 })
</script>

<template>
  <div
    role="alert"
    class="flex w-full items-center gap-3 rounded-lg bg-tint-danger px-3.5 py-2.5 text-on-danger"
  >
    <!-- The icon marks the first line; the controls sit centred on the whole bar. -->
    <span class="flex h-5 shrink-0 items-center self-start" aria-hidden="true">
      <CircleX :size="14" />
    </span>
    <div class="min-w-0 flex-1 select-text">
      <p class="text-chrome leading-5">{{ label }}</p>
      <p
        v-if="detail"
        class="mt-0.5 whitespace-pre-line break-words text-[12px] leading-[18px] text-on-danger-muted"
      >
        {{ detail }}
      </p>
      <p
        v-if="facts.length > 0"
        class="mt-1.5 truncate font-mono text-[11px] leading-4 text-on-danger-muted"
      >
        {{ facts.join(' · ') }}
      </p>
    </div>
    <div
      v-if="copyText || action"
      class="flex shrink-0 items-center gap-1"
    >
      <Tooltip
        v-if="copyText"
        :content="copied ? t('common.copied') : t('common.copy')"
      >
        <IconButton
          :icon="copied ? Check : Copy"
          size="sm"
          variant="ghost"
          :aria-label="t('common.copy')"
          @click="copy(props.copyText ?? '')"
        />
      </Tooltip>
      <Button v-if="action" size="sm" @click="$emit('action')">{{
        action
      }}</Button>
    </div>
  </div>
</template>
