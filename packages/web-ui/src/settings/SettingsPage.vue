<script setup lang="ts">
import { provide, ref } from 'vue'
import type { SentenceText, TitleText } from '../ui/ui-text'
import { settingsPageKey } from './navigation'
/**
 * One settings section: a title, an optional line under it, then groups.
 * While a level inside it is open on a narrow dialog (a provider beside the
 * list of providers), that level is the page and its title, so this title
 * steps aside.
 */
const props = defineProps<{
  title: TitleText
  description?: SentenceText
  /** Use the whole column: for a list beside its detail. */
  wide?: boolean
  /**
   * Fill the host's height so a child can scroll on its own instead of the
   * page; a narrow dialog's page scrolls as a whole, with no scroll region
   * inside another.
   */
  fill?: boolean
}>()

const nested = ref(false)
provide(settingsPageKey, { title: () => props.title, nested })
</script>

<template>
  <div
    class="mx-auto flex w-full flex-col gap-8"
    :class="[wide ? 'max-w-none' : 'max-w-2xl', fill ? '@md:h-full @md:min-h-0' : '']"
  >
    <header class="select-none" :class="nested ? 'hidden @md:block' : ''">
      <h2 class="text-[20px] font-medium leading-7 text-fg-emphasis">{{ title }}</h2>
      <p v-if="description" class="mt-1 text-[13px] leading-5 text-fg-muted">{{ description }}</p>
    </header>
    <slot />
  </div>
</template>
