<script setup lang="ts">
import { KEEPS_WINDOWS } from '../../ui/window-stack'
import { t } from '../locales/text'

/**
 * What a notice row says (`crates/agent-session/src/reports.rs`): what
 * happened, then what it happened to by its title.
 *
 * The title is the notice's object (an imperative title read as the subject
 * reads wrongly). It is underlined with dots and comes after a colon, or, for a
 * title that ends the sentence, such as *main agent* in *Message from main
 * agent*, after the words alone.
 *
 * A click on the title brings into view where it came from, as a control of its
 * own. A title nothing can show is plain text. Where it came from may lie in a
 * child's transcript, which the Agents window opens on or switches to, so the
 * title is that window's control, as its chip is: a press on it is not one
 * outside the session's windows (`KEEPS_WINDOWS`).
 *
 * On a narrow row the title gives way first, down to a few characters, and then
 * the words, each ending in an ellipsis.
 */
defineProps<{
  notice: string
  /** What the notice names; none for a notice that names nothing. */
  title?: string | null
  /**
   * The title stands inside the notice's sentence, with no colon between; the
   * words keep their own spaces.
   */
  inline?: boolean
  /** What follows an inline title in the sentence. */
  after?: string
  /** Brings where the title came from into view; none when nothing can. */
  go?: (() => void) | undefined
}>()
</script>

<template>
  <span v-if="title == null" class="min-w-0 truncate">{{ notice }}</span>
  <span
    v-else
    class="notice-label flex min-w-0 items-center"
    :class="inline ? 'notice-label-inline' : ''"
  >
    <span v-if="notice" class="notice-words">{{ inline ? notice : t('transcript.notice.titled', { notice }) }}</span>
    <span class="notice-title">
      <!-- The title's click is its own, never the row's. -->
      <button
        v-if="go"
        type="button"
        class="notice-title-text cursor-pointer text-fg-muted underline decoration-dotted decoration-fg-faint underline-offset-3 transition-colors duration-200 ease-out hover:text-fg-body hover:decoration-current"
        v-bind="{ [KEEPS_WINDOWS]: '' }"
        @click.stop="go"
      >{{ title }}</button>
      <span v-else class="notice-title-text text-fg-muted">{{ title }}</span>
    </span>
    <span v-if="after" class="notice-words">{{ after }}</span>
  </span>
</template>

<style scoped>
/*
 * Only the title shrinks, so the words keep their whole width while the title
 * gives way. The words are capped to leave the title a few characters, and give
 * way too once the row is narrower than that. The title is never narrower than
 * those characters: a row is as wide as its content, so a shorter title, such
 * as *Agent 1*, would otherwise leave the words less than their own width.
 */
.notice-label {
  --notice-gap: calc(var(--spacing) * 2);
  --notice-title-keeps: 6ch;
  gap: var(--notice-gap);
}

/*
 * The words of an inline sentence keep their own spaces, as the language writes
 * them.
 */
.notice-label-inline {
  --notice-gap: 0;
}

.notice-label-inline .notice-words {
  white-space: pre;
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
  min-width: var(--notice-title-keeps);
}

.notice-title-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
