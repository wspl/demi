<script setup lang="ts">
/**
 * A reference to another part of the page inside a line of text, such as
 * the command a call names: underlined with dots, as a reference. A
 * reference that can be followed is a link; following it calls `follow`
 * and goes no further, so a row it sits in does not take the click too.
 * Without `follow` it is the text alone. It truncates at its end when the
 * line has no room, so put it in a flex row with `min-w-0` around it.
 */
defineProps<{
  text: string
  follow?: () => void
}>()
</script>

<template>
  <span
    v-if="follow"
    role="link"
    tabindex="0"
    class="min-w-0 cursor-pointer truncate underline decoration-dotted decoration-from-font underline-offset-[3px] transition-colors duration-200 ease-out hover:text-fg-emphasis"
    @click.stop="follow()"
    @keydown.enter.stop.prevent="follow()"
  >{{ text }}</span>
  <span v-else class="min-w-0 truncate">{{ text }}</span>
</template>
