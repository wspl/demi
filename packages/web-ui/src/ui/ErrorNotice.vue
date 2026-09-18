<script setup lang="ts">
import { ref } from 'vue'
import { useClipboard } from '@vueuse/core'
import { Check, CircleX, Copy } from '@lucide/vue'
import { t } from '../infra/i18n'
import Button from './Button.vue'
import Fold from './Fold.vue'
import FoldChevron from './FoldChevron.vue'
import IconButton from './IconButton.vue'
import { ICON_PX } from './icon-metrics'
import Tooltip from './Tooltip.vue'

/**
 * A failure told in the flow of the conversation: a turn that failed, a
 * message that was not delivered, a session that lost its connection.
 *
 * One tinted bar, full width where it sits: a sentence, the upstream message
 * under it, the facts on one line, the source's payload behind a fold, and at
 * most one action beside a copy control for the support thread. It is a record in the transcript, not a
 * fixture of the dock, so it scrolls with what it describes.
 */
const props = withDefaults(
  defineProps<{
    label: string
    /** The upstream message or reason, in the caller's words. */
    detail?: string | null
    /** Short facts under the message, in the UI font: what the reader does next, such as when a limit lifts. */
    facts?: readonly string[]
    /** What the source sent, in full, behind a disclosure: the vendor's own payload. */
    raw?: string | null
    /** The single action's label; the caller decides what it does. */
    action?: string
    /** What the copy control puts on the clipboard; no control without it. */
    copyText?: string | null
  }>(),
  {
    detail: null,
    raw: null,
    facts: () => [],
    copyText: null,
  },
)
defineEmits<{ action: [] }>()
const { copy, copied } = useClipboard({ copiedDuring: 1500 })
const rawOpen = ref(false)
</script>

<template>
  <div
    role="alert"
    class="flex w-full items-center gap-3 rounded-lg bg-tint-danger px-3.5 py-2.5 text-on-danger"
  >
    <!-- The icon holds the first line of the text, whatever the bar's height; the
         text with its icon sits centred against the controls. -->
    <div class="flex min-w-0 flex-1 items-start gap-3">
      <span class="flex h-5 shrink-0 items-center" aria-hidden="true">
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
          class="mt-1 truncate text-[12px] leading-[18px] text-on-danger-muted"
        >
          {{ facts.join(' · ') }}
        </p>
        <template v-if="raw">
          <button
            type="button"
            class="mt-1 flex cursor-default items-center gap-1 text-[12px] leading-[18px] text-on-danger-muted hover:text-on-danger"
            :aria-expanded="rawOpen"
            @click="rawOpen = !rawOpen"
          >
            <!-- The label, then its chevron, as the transcript's foldable rows read ("Thought briefly ›"). -->
            {{ t('error.upstream') }}
            <FoldChevron :open="rawOpen" :size="ICON_PX.in12" />
          </button>
          <!-- Mounted while closed so the height can animate both ways. -->
          <Fold :open="rawOpen">
            <pre
              class="mt-1 max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-md bg-sunken p-2 font-mono text-[11px] leading-4 text-on-danger-muted"
            >{{ raw }}</pre>
          </Fold>
        </template>
      </div>
    </div>
    <!-- Two lines: the controls sit centred. Three (with facts): they hold the first line. -->
    <div
      v-if="copyText || action"
      class="flex shrink-0 items-center gap-1"
      :class="facts.length > 0 ? '-mt-0.5 self-start' : ''"
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
