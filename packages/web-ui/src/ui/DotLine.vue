<script setup lang="ts">
/**
 * Short facts on a line, a dot between each. A line too narrow for them
 * breaks between two facts, never inside one, and no line starts with a
 * dot: each fact's dot hangs before it, and the first on a line hangs past
 * the edge, where it is clipped.
 */
defineProps<{
  parts: readonly string[]
}>()
</script>

<template>
  <div class="overflow-hidden">
    <div class="-ml-3 flex flex-wrap">
      <span
        v-for="(part, index) in parts"
        :key="index"
        class="whitespace-nowrap before:inline-block before:w-3 before:text-center before:content-['·']"
      >{{ part }}</span>
    </div>
  </div>
</template>
