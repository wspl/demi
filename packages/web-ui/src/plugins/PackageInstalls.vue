<script setup lang="ts">
import ProgressBar from '../ui/ProgressBar.vue'
import { formatBytes } from '../files/format'
import type { PackageInstall } from './client'

/**
 * What a Host installs before a plugin's first call can run
 * (`native-runtime.md` § Installation progress): each artifact with its
 * phase and bytes, such as `Installing demi.browser: Chrome for Testing
 * 153.0.8010.36`, `120 MB of 196 MB`.
 */
defineProps<{ installs: readonly PackageInstall[] }>()

function artifact(install: PackageInstall): string {
  return `${install.name} ${install.version}`
}

function progress(install: PackageInstall): string {
  if (install.phase === 'unpack') {
    return 'Unpacking…'
  }
  return `${formatBytes(install.done)} of ${formatBytes(install.total)}`
}
</script>

<template>
  <ul class="flex w-full max-w-80 flex-col gap-3 text-left">
    <li
      v-for="install in installs"
      :key="`${install.package} ${install.name}`"
      class="flex flex-col gap-1.5"
    >
      <span class="text-fg-subtle [overflow-wrap:anywhere]">Installing {{ install.package }}: {{ artifact(install) }}</span>
      <ProgressBar
        :value="install.done"
        :max="install.total"
        :label="`Installing ${install.package}: ${artifact(install)}`"
      />
      <span class="text-[12px] tabular-nums text-fg-faint">{{ progress(install) }}</span>
    </li>
  </ul>
</template>
