<script setup lang="ts">
import { Sun, Moon } from '@lucide/vue'
import { useTheme, toggleTheme } from '../theme/appTheme'
import IconButton from './IconButton.vue'
import Tooltip from './Tooltip.vue'

const { theme } = useTheme()
</script>

<template>
  <Tooltip :content="theme === 'dark' ? 'Light theme' : 'Dark theme'">
    <IconButton
      :icon="theme === 'dark' ? Sun : Moon"
      :aria-label="theme === 'dark' ? 'Light theme' : 'Dark theme'"
      @click="toggleTheme()"
    />
  </Tooltip>
</template>
