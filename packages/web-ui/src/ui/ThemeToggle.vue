<script setup lang="ts">
import { Sun, Moon } from '@lucide/vue'
import { useTheme, toggleTheme } from '../theme/appTheme'
import IconButton from './IconButton.vue'

const { theme } = useTheme()
</script>

<template>
  <IconButton
    :icon="theme === 'dark' ? Sun : Moon"
    :aria-label="theme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme'"
    :title="theme === 'dark' ? 'Light mode' : 'Dark mode'"
    @click="toggleTheme()"
  />
</template>
