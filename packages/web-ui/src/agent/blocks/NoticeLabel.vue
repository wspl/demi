<script setup lang="ts">
import Tooltip from '../../ui/Tooltip.vue'
import { KEEPS_WINDOWS } from '../../ui/window-stack'

/**
 * What a notice row says (`runtime.md` § Command reports, `subagents.md`
 * § Product rendering): what happened, then what it happened to by its
 * title, the notice's object, underlined with dots; or, for a title that
 * can be the subject, such as *Main agent*, the title first. A click on the
 * title brings into view where it came from, as a control of its own; a
 * title nothing can show is plain text. Where it came from may lie in a
 * child's transcript, which the Agents window opens on or switches to, so
 * the title is that window's control, as its chip is: a press on it is not
 * one outside the session's windows (`KEEPS_WINDOWS`, `product.md` § the
 * dock's chips). On a narrow row the title gives way
 * first, down to a few characters, and then the words, each ending in an
 * ellipsis. The `tip` slot, when given, is what the title shows pointed at,
 * such as what an agent runs on.
 */
defineSlots<{ tip?(): unknown }>()
defineProps<{
  notice: string
  /** What the notice names; none for a notice that names nothing. */
  title?: string | null
  /** The title is the notice's subject and leads it. */
  subject?: boolean
  /** Brings where the title came from into view; none when nothing can. */
  go?: (() => void) | undefined
}>()
</script>

<template>
  <span v-if="title == null" class="min-w-0 truncate">{{ notice }}</span>
  <span
    v-else
    class="notice-label flex min-w-0 items-center"
    :class="subject ? 'notice-label-subject' : ''"
  >
    <span v-if="!subject" class="notice-words">{{ notice }}:</span>
    <Tooltip class="notice-title" :disabled="!$slots.tip">
      <!-- The title's click is its own, never the row's. -->
      <button
        v-if="go"
        type="button"
        class="notice-title-text cursor-pointer text-fg-muted underline decoration-dotted decoration-fg-faint underline-offset-3 transition-colors duration-200 ease-out hover:text-fg-body hover:decoration-current"
        v-bind="{ [KEEPS_WINDOWS]: '' }"
        @click.stop="go"
      >{{ title }}</button>
      <span v-else class="notice-title-text text-fg-muted">{{ title }}</span>
      <template v-if="$slots.tip" #overlay>
        <slot name="tip" />
      </template>
    </Tooltip>
    <span v-if="subject" class="notice-words">{{ notice }}</span>
  </span>
</template>

<style scoped>
/*
 * Only the title shrinks, so the words keep their whole width while the
 * title gives way; the words are capped to leave the title a few
 * characters, and give way too once the row is narrower than that.
 */
.notice-label {
  --notice-gap: calc(var(--spacing) * 2);
  --notice-title-keeps: 6ch;
  gap: var(--notice-gap);
}

/* A subject and its predicate are one sentence, a space apart. */
.notice-label-subject {
  --notice-gap: 0.25em;
}

.notice-words {
  flex-shrink: 0;
  max-width: max(0px, calc(100% - var(--notice-title-keeps) - var(--notice-gap)));
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.notice-title {
  display: flex;
  min-width: 0;
}

.notice-title-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
