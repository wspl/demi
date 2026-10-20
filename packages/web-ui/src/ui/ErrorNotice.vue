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
 * A failure told in the flow of the conversation: a turn that failed, a message
 * that was not delivered, a session that lost its connection.
 *
 * One tinted bar, full width where it sits, holding in order:
 *
 * - a sentence,
 * - the upstream message under it,
 * - the facts on a line of their own, wrapping where it is narrow,
 * - a control of the record's own under them (the default slot),
 * - the source's payload behind a fold, and
 * - its actions, most often one, the first the one to try first, beside a copy
 *   control for the support thread.
 *
 * It is a record in the transcript, not a fixture of the dock, so it scrolls with
 * what it describes.
 */
export interface NoticeAction {
  label: TitleText
  run: () => void
}

withDefaults(
  defineProps<{
    label: SentenceText
    /** The upstream message or reason, in the caller's words. */
    detail?: string | null
    /** Short facts under the message, in the UI font: what the reader does next, such as when a limit lifts. */
    facts?: readonly SentenceText[]
    /** What the source sent, in full, behind a disclosure: the vendor's own payload. */
    raw?: string | null
    /** The response's headers, under the payload, folded under Headers: they rarely say why. */
    rawHeaders?: string | null
    /** What the reader can do about it, in the order they are offered. */
    actions?: readonly NoticeAction[]
    /** What the copy control puts on the clipboard; no control without it. */
    copyText?: string | null
    /** The request the failure came from, named under the message. */
    requestId?: string | null
  }>(),
  {
    detail: null,
    raw: null,
    rawHeaders: null,
    facts: () => [],
    actions: () => [],
    copyText: null,
    requestId: null,
  },
)
const rawOpen = ref(false)
const headersOpen = ref(false)
</script>

<template>
  <!-- The text and the controls share a line while both fit; on a narrow
       bar the controls go under the text, from its left edge, and the text
       keeps room for about twenty characters a line. -->
  <div
    role="alert"
    class="flex w-full flex-wrap items-center gap-x-3 gap-y-2 rounded-lg bg-tint-danger px-3.5 py-2.5 text-on-danger"
  >
    <!-- The icon holds the first line of the text, whatever the bar's height; the
         text with its icon sits centred against the controls. -->
    <div class="flex min-w-[calc(26px+20ch)] flex-1 items-start gap-3">
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
              <template v-if="rawHeaders">
                <button
                  type="button"
                  class="mt-2 flex cursor-default items-center gap-1 text-detail leading-[18px] text-on-danger-muted hover:text-on-danger"
                  :aria-expanded="headersOpen"
                  @click="headersOpen = !headersOpen"
                >
                  {{ t('errorNotice.headers') }}
                  <FoldChevron :open="headersOpen" :size="ICON_PX.in12" />
                </button>
                <Fold :open="headersOpen">
                  <pre
                    class="mt-1 whitespace-pre-wrap break-words font-mono text-[11px] leading-4 text-on-danger-muted"
                  >{{ rawHeaders }}</pre>
                </Fold>
              </template>
            </ScrollArea>
          </Fold>
        </template>
      </div>
    </div>
    <!-- Indented by the icon and its gap (14px + 12px), so that under the
         text they start where it does. Two lines: the controls sit centred. Three (with facts): they hold the first line. -->
    <div
      v-if="copyText || actions.length > 0"
      class="on-fill flex shrink-0 items-center gap-1 pl-[26px]"
      :class="facts.length > 0 ? '-mt-0.5 self-start' : ''"
    >
      <CopyButton v-if="copyText" :source="copyText" />
      <Button
        v-for="action in actions"
        :key="action.label"
        size="sm"
        @click="action.run()"
      >{{ action.label }}</Button>
    </div>
  </div>
</template>
