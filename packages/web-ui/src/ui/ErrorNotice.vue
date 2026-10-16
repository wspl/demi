<script setup lang="ts">
import { ref } from 'vue'
import { CircleX } from '@lucide/vue'
import Button from './Button.vue'
import Fold from './Fold.vue'
import FoldChevron from './FoldChevron.vue'
import CopyButton from './CopyButton.vue'
import RequestIdLine from './RequestIdLine.vue'
import ScrollArea from './ScrollArea.vue'
import { t } from './locales/text'
import { ICON_PX } from './icon-metrics'
import type { SentenceText, TitleText } from './ui-text'

/**
 * A failure told in the flow of the conversation: a turn that failed, a
 * message that was not delivered, a session that lost its connection.
 *
 * One tinted bar, full width where it sits: a sentence, the upstream message
 * under it, the facts on a line of their own, wrapping where it is narrow, a control of the record's own under them
 * (the default slot), the source's payload behind a fold, and at most one
 * action beside a copy control for the support thread. It is a record in the transcript, not a
 * fixture of the dock, so it scrolls with what it describes.
 */
withDefaults(
  defineProps<{
    label: SentenceText
    /** The upstream message or reason, in the caller's words. */
    detail?: string | null
    /** Short facts under the message, in the UI font: what the reader does next, such as when a limit lifts. */
    facts?: readonly SentenceText[]
    /** What the source sent, in full, behind a disclosure: the vendor's own payload. */
    raw?: string | null
    /** The single action's label; the caller decides what it does. */
    action?: TitleText
    /** What the copy control puts on the clipboard; no control without it. */
    copyText?: string | null
    /** The request the failure came from, named under the message. */
    requestId?: string | null
  }>(),
  {
    detail: null,
    raw: null,
    facts: () => [],
    copyText: null,
    requestId: null,
  },
)
defineEmits<{ action: [] }>()
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
          class="mt-0.5 whitespace-pre-line break-words text-detail leading-[18px] text-on-danger-muted"
        >
          {{ detail }}
        </p>
        <!-- Facts wrap on a narrow screen: a time cut off says nothing. -->
        <p
          v-if="facts.length > 0"
          class="mt-1 break-words text-detail leading-[18px] text-on-danger-muted"
        >
          {{ facts.join(' · ') }}
        </p>
        <!-- The one control that belongs to the record itself, such as a preference its facts name. -->
        <slot />
        <RequestIdLine
          v-if="requestId"
          class="mt-0.5 text-on-danger-muted"
          :request-id="requestId"
        />
        <template v-if="raw">
          <button
            type="button"
            class="mt-1 flex cursor-default items-center gap-1 text-detail leading-[18px] text-on-danger-muted hover:text-on-danger"
            :aria-expanded="rawOpen"
            @click="rawOpen = !rawOpen"
          >
            <!-- The label, then its chevron, as the transcript's foldable rows read ("Thought briefly ›"). -->
            {{ t('errorNotice.providerResponse') }}
            <FoldChevron :open="rawOpen" :size="ICON_PX.in12" />
          </button>
          <!-- Mounted while closed so the height can animate both ways. -->
          <Fold :open="rawOpen">
            <ScrollArea class="mt-1 max-h-64 rounded-md bg-sunken" viewport-class="p-2">
              <pre
                class="whitespace-pre-wrap break-words font-mono text-[11px] leading-4 text-on-danger-muted"
              >{{ raw }}</pre>
            </ScrollArea>
          </Fold>
        </template>
      </div>
    </div>
    <!-- Two lines: the controls sit centred. Three (with facts): they hold the first line. -->
    <div
      v-if="copyText || action"
      class="on-fill flex shrink-0 items-center gap-1"
      :class="facts.length > 0 ? '-mt-0.5 self-start' : ''"
    >
      <CopyButton v-if="copyText" :source="copyText" />
      <Button v-if="action" size="sm" @click="$emit('action')">{{
        action
      }}</Button>
    </div>
  </div>
</template>
