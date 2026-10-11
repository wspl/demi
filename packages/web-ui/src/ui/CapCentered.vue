<script setup lang="ts">
/**
 * Centers a mark, a status dot or a chevron, on the middle of the capitals
 * of the text beside it, as an all-caps label needs: capitals have no
 * descenders, so a mark centered on the line box sits below them.
 *
 * The box is one cap height tall (the font's `cap` unit) and stands on the
 * text's baseline, which the parent's baseline alignment gives it
 * (`items-baseline`); the mark is centered in it. The baseline is the one the
 * text is painted on, so the mark follows the text whatever the font and
 * however the browser rounds the line.
 */
</script>

<template>
  <span class="flex h-[1cap] items-center">
    <!-- A box without text has its baseline at its bottom; this one gives the whole box its baseline. -->
    <span class="h-full self-baseline" aria-hidden="true" />
    <slot />
  </span>
</template>
