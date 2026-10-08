<script setup lang="ts">
import { useSlots } from 'vue'
import TruncatedText from './TruncatedText.vue'

/**
 * One row of a `ListCard`: an icon in a small tile, the thing's name at the
 * chrome size over a muted detail line, and one control at its end. The row
 * owns the insets at both ends: the tile and the control each keep as much
 * room to the row's edge as centering gives them above and below,
 * (row height - their height) / 2, so a caller puts a control in the
 * `action` slot and never positions it.
 */
const props = withDefaults(defineProps<{
  title: string
  detail?: string
  /** The size of the Button or IconButton in the `action` slot. */
  actionSize?: 'xs' | 'sm' | 'md'
}>(), {
  detail: undefined,
  actionSize: 'sm',
})

const slots = useSlots()

const ACTION_HEIGHT = {
  xs: 'var(--spacing-hit-xs)',
  sm: 'var(--spacing-hit-sm)',
  md: 'var(--spacing-hit)',
} as const
</script>

<template>
  <div
    class="list-card-row flex min-w-0 items-center gap-2.5"
    :class="slots['action'] ? '' : 'list-card-row-end'"
    :style="{ '--list-card-action-h': ACTION_HEIGHT[props.actionSize] }"
    role="listitem"
  >
    <span
      v-if="slots['icon']"
      class="list-card-row-tile inline-flex shrink-0 items-center justify-center rounded-md bg-overlay/8 text-fg-muted"
    >
      <slot name="icon" />
    </span>
    <div class="flex min-w-0 flex-1 flex-col">
      <TruncatedText :text="title" class="text-chrome leading-5 font-medium text-fg-body" />
      <TruncatedText v-if="detail" :text="detail" class="text-xs leading-4 text-fg-muted" />
    </div>
    <span v-if="slots['action']" class="list-card-row-action">
      <slot name="action" />
    </span>
  </div>
</template>

<style scoped>
/* 52px: two lines of 20px and 16px with 8px above and below. The tile is a
   hit-sized square, so it keeps 12px to the row's top, bottom and left.
   The height is the content's: the list's hairline between rows adds to it. */
.list-card-row {
  --list-card-row-h: 52px;
  --list-card-tile: var(--spacing-hit);
  box-sizing: content-box;
  height: var(--list-card-row-h);
  padding-left: calc((var(--list-card-row-h) - var(--list-card-tile)) / 2);
}

/* Without a control, the text keeps the tile's inset at the other end. */
.list-card-row-end {
  padding-right: calc((var(--list-card-row-h) - var(--list-card-tile)) / 2);
}

.list-card-row-tile {
  width: var(--list-card-tile);
  height: var(--list-card-tile);
}

/* The control sits as far from the row's right edge as centering puts it
   from its top and bottom: (row height - control height) / 2. */
.list-card-row-action {
  display: flex;
  flex-shrink: 0;
  /* No inline strut, so the Tooltip's wrapper cannot lift the control off center. */
  line-height: 0;
  margin-right: calc((var(--list-card-row-h) - var(--list-card-action-h)) / 2);
}
</style>
