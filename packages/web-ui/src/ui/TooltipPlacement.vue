<script setup lang="ts">
import type { Placement } from '@floating-ui/vue'
import { provideTooltipPlacement } from './tooltip-placement'

/**
 * Its content's tips open on `placement` unless they name their own side,
 * such as the controls of a row right under a tab strip, whose tips open
 * below so none covers the strip's tabs.
 */
const props = defineProps<{ placement: Placement }>()
provideTooltipPlacement(props.placement)
</script>

<template>
  <slot />
</template>
