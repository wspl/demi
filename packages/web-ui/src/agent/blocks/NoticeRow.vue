<script setup lang="ts">
import { useSlots } from 'vue'
import FunctionalBlock from './FunctionalBlock.vue'

/**
 * A notice row in the transcript, for a command's end and an agent's
 * receipt alike (`runtime.md` § Command reports, `subagents.md` § Product
 * rendering): what happened, in the user's words, then what it happened
 * to by its title, the notice's object, underlined with dots. A click on
 * the title brings into view where it came from; a title nothing can show
 * is plain text. After the title, a tag marks what went wrong. A notice
 * that names nothing, such as one of the user's own decisions, shows its
 * words alone. A body makes the row open to read it.
 */
defineProps<{
  notice: string
  /** What the notice names; none for a notice that names nothing. */
  title?: string | null
  /** Brings where the title came from into view; none when nothing can. */
  go?: (() => void) | undefined
}>()
const isOpen = defineModel<boolean>('open', { default: false })
const slots = useSlots()
</script>

<template>
  <!-- Slot presence is not reactive, so the row says it opens rather than FunctionalBlock inferring it. -->
  <FunctionalBlock v-model:open="isOpen" :expandable="!!slots['body']">
    <template #icon>
      <slot name="icon" />
    </template>
    <span v-if="title == null" class="min-w-0 truncate">{{ notice }}</span>
    <template v-else>
      <span class="shrink-0">{{ notice }}:</span>
      <!-- The title is a link, which a long title keeps while it truncates; its click is its own, never the row's. -->
      <button
        v-if="go"
        type="button"
        class="min-w-0 cursor-pointer truncate text-fg-muted underline decoration-dotted decoration-fg-faint underline-offset-3 transition-colors duration-200 ease-out hover:text-fg-body hover:decoration-current"
        @click.stop="go"
      >{{ title }}</button>
      <span v-else class="min-w-0 truncate text-fg-muted">{{ title }}</span>
    </template>
    <slot name="tag" />
    <template v-if="slots['body']" #body>
      <slot name="body" />
    </template>
  </FunctionalBlock>
</template>
