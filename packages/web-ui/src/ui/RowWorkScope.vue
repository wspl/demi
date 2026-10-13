<script setup lang="ts">
import { provideRowWorkScope, type RowWork } from './pending-action'

/**
 * The row a menu acts on, for a menu that serves many rows (`createKeyedRowWork`):
 * the icon buttons and menu items in its slot hand their work to `scope`, and
 * the row shows it.
 */
const props = defineProps<{
  scope: RowWork
}>()

provideRowWorkScope(() => props.scope)
</script>

<template>
  <slot />
</template>
