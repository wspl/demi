<script setup lang="ts">
import FunctionalBlock from './FunctionalBlock.vue'
import NoticeLabel from './NoticeLabel.vue'

/**
 * A notice row in the transcript, for a command's end and an agent's
 * receipt alike (`runtime.md` § Command reports, `subagents.md` § Product
 * rendering), its words as `NoticeLabel` says them. After them, a tag
 * marks what went wrong. A notice that names nothing, such as one of the
 * user's own decisions, shows its words alone. It does not open: no
 * chevron, no body, no toggle; its title, the one control, leads where the
 * detail is.
 */
defineProps<{
  notice: string
  /** What the notice names; none for a notice that names nothing. */
  title?: string | null
  /** The title is the notice's subject and leads it. */
  subject?: boolean
  /** Brings where the title came from into view; none when nothing can. */
  go?: (() => void) | undefined
  /** The row is the turn's activity, as a run's row is, and shimmers. */
  loading?: boolean
}>()
</script>

<template>
  <FunctionalBlock :expandable="false" :loading="loading">
    <template #icon>
      <slot name="icon" />
    </template>
    <NoticeLabel :notice="notice" :title="title" :subject="subject" :go="go" />
    <slot name="tag" />
  </FunctionalBlock>
</template>
