<script setup lang="ts">
import ProgressBar from '../ui/ProgressBar.vue'
import { formatBytes } from '../files/format'
import type { HostInstall } from './installs'

/**
 * What Hosts install while someone waits on them (`native-runtime.md`
 * § Installation progress): each artifact with its phase and bytes, such as
 * `Installing demi.browser: Chrome for Testing 153.0.8010.36`,
 * `120 MB of 196 MB`. It takes its container's width.
 */
defineProps<{ installs: readonly HostInstall[] }>()

function artifact(install: HostInstall): string {
  return `${install.name} ${install.version}`
}

function progress(install: HostInstall): string {
  if (install.phase === 'unpack') {
    return 'Unpacking…'
  }
  return `${formatBytes(install.done)} of ${formatBytes(install.total)}`
}
</script>

<template>
  <ul class="flex w-full flex-col gap-3 text-left text-[13px]">
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
