<script setup lang="ts">
/**
 * What a notice row says (`runtime.md` § Command reports, `subagents.md`
 * § Product rendering): what happened, then what it happened to by its
 * title, the notice's object, underlined with dots; or, for a title that
 * can be the subject, such as *Main agent*, the title first. A click on the
 * title brings into view where it came from, as a control of its own; a
 * title nothing can show is plain text. On a narrow row the title gives way
 * first, down to a few characters, and then the words, each ending in an
 * ellipsis.
 */
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
    <!-- The title's click is its own, never the row's. -->
    <button
      v-if="go"
      type="button"
      class="notice-title cursor-pointer text-fg-muted underline decoration-dotted decoration-fg-faint underline-offset-3 transition-colors duration-200 ease-out hover:text-fg-body hover:decoration-current"
      @click.stop="go"
    >{{ title }}</button>
    <span v-else class="notice-title text-fg-muted">{{ title }}</span>
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
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
