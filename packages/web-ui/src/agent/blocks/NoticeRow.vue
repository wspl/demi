<script setup lang="ts">
import { useSlots } from 'vue'
import FunctionalBlock from './FunctionalBlock.vue'
import NoticeLabel from './NoticeLabel.vue'

/**
 * A notice row in the transcript, for a command's end and an agent's
 * receipt alike (`runtime.md` § Command reports, `subagents.md` § Product
 * rendering), its words as `NoticeLabel` says them. After them, a tag
 * marks what went wrong. A notice that names nothing, such as one of the
 * user's own decisions, shows its words alone. A body makes the row open to
 * read it: the row's toggle and the title's link are then two controls side
 * by side, each with its own focus.
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
const isOpen = defineModel<boolean>('open', { default: false })
const slots = useSlots()
</script>

<template>
  <!-- Slot presence is not reactive, so the row says it opens rather than FunctionalBlock inferring it. -->
  <FunctionalBlock v-model:open="isOpen" :expandable="!!slots['body']" :loading="loading">
    <template #icon>
      <slot name="icon" />
    </template>
    <NoticeLabel :notice="notice" :title="title" :subject="subject" :go="go" />
    <slot name="tag" />
    <template v-if="slots['body']" #body>
      <slot name="body" />
    </template>
  </FunctionalBlock>
</template>
