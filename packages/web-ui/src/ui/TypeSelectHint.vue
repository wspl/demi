<script setup lang="ts">
/**
 * What a type-select query has typed so far (`useTypeSelect`), floating at
 * the top-right of the list, tree or menu it searches, over the rows and
 * without moving them; faint while no row matches. The host is positioned.
 */
defineProps<{
  query: string
  matched: boolean
}>()
</script>

<template>
  <Transition
    enter-active-class="transition-opacity duration-120 ease-out"
    leave-active-class="transition-opacity duration-100 ease-out"
    enter-from-class="opacity-0"
    leave-to-class="opacity-0"
  >
    <div
      v-if="query"
      class="overlay-shell pointer-events-none absolute right-2 top-1 z-[3] max-w-[calc(100%-1rem)] select-none truncate whitespace-pre rounded-md px-2 py-1 text-[12px] leading-4"
      :class="matched ? 'text-fg' : 'text-fg-faint'"
      aria-hidden="true"
    >{{ query }}</div>
  </Transition>
</template>
