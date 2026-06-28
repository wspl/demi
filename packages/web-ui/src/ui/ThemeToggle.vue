<script setup lang="ts">
import { useTheme, toggleTheme } from '../theme/appTheme'

const { theme } = useTheme()
</script>

<template>
  <button
    type="button"
    class="flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-md text-fg-subtle transition-colors hover:bg-hover hover:text-fg-body"
    :aria-label="theme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme'"
    :title="theme === 'dark' ? 'Light mode' : 'Dark mode'"
    @click="toggleTheme()"
  >
    <!-- Sun: shown in dark mode (click to go light) -->
    <svg
      v-if="theme === 'dark'"
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="4" />
      <path
        d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41"
      />
    </svg>
    <!-- Moon: shown in light mode (click to go dark) -->
    <svg
      v-else
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
    </svg>
  </button>
</template>
