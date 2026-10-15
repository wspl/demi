<script setup lang="ts">
import { computed } from 'vue'
import { Sun, Moon } from '@lucide/vue'
import { useTheme, toggleTheme } from '../theme/appTheme'
import IconButton from './IconButton.vue'
import Tooltip from './Tooltip.vue'
import { t } from './locales/text'

const { theme } = useTheme()
const label = computed(() => (theme.value === 'dark' ? t('themeToggle.light') : t('themeToggle.dark')))
</script>

<template>
  <Tooltip :content="label">
    <IconButton
      :icon="theme === 'dark' ? Sun : Moon"
      :aria-label="label"
      @click="toggleTheme()"
    />
  </Tooltip>
</template>
