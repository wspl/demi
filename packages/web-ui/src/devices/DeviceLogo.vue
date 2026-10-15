<script setup lang="ts">
import { DEVICE_LOGO_PATH, type DeviceLogo } from './mark'

/** A device's system's logo at `size`, in the text's color: the logo of its mark (`DeviceIcon`). */
defineProps<{
  logo: DeviceLogo
  size: number
}>()
</script>

<template>
  <svg viewBox="0 0 24 24" :width="size" :height="size" class="shrink-0" aria-hidden="true">
    <path :d="DEVICE_LOGO_PATH[logo]" fill="currentColor" />
  </svg>
</template>
