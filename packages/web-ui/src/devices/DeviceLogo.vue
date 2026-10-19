<script setup lang="ts">
import { DEVICE_LOGO_PATH, type SystemLogo } from './mark'

/**
 * A system's logo at `size`, in the text's color: the logo of a device's mark (`DeviceIcon`), or
 * of a simulator's system, which its tab in the work panel shows.
 */
defineProps<{
  logo: SystemLogo
  size: number
}>()
</script>

<template>
  <svg viewBox="0 0 24 24" :width="size" :height="size" class="shrink-0" aria-hidden="true">
    <path :d="DEVICE_LOGO_PATH[logo]" fill="currentColor" />
  </svg>
</template>
