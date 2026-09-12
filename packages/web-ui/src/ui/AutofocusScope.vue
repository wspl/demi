<script setup lang="ts">
import { computed, provide } from 'vue'
import { autofocusEnabledKey } from './autofocus'

const props = defineProps<{ enabled: boolean }>()
provide(
  autofocusEnabledKey,
  computed(() => props.enabled),
)
</script>

<template>
  <slot />
</template>
