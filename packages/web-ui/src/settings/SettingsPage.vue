<script setup lang="ts">
import { inject, onBeforeUnmount, provide, ref, watch } from 'vue'
import { ChevronLeft } from '@lucide/vue'
import Button from '../ui/Button.vue'
import { ICON_PX } from '../ui/icon-metrics'
import type { SentenceText, TitleText } from '../ui/ui-text'
import { settingsLevelKey, settingsPageKey, type SettingsLevel } from './navigation'
/**
 * One settings section: a title, an optional line under it, then groups.
 * While a level inside it is open on a narrow dialog (a provider beside the
 * list of providers), that level is the page and its title, so this title
 * steps aside. A page a section opened, such as a device's beside the list
 * of devices, names the page it returns to in `back`: a back button above
 * its title says so, and on a narrow dialog the dialog's own navigation bar
 * goes back to it instead, as System Settings and iOS do.
 */
const props = defineProps<{
  title: TitleText
  description?: SentenceText
  /** The title of the page this one returns to: Devices. */
  back?: TitleText
  /** Use the whole column: for a list beside its detail. */
  wide?: boolean
  /**
   * Fill the host's height so a child can scroll on its own instead of the
   * page; a narrow dialog's page scrolls as a whole, with no scroll region
   * inside another.
   */
  fill?: boolean
}>()
const emit = defineEmits<{
  back: []
}>()

const nested = ref(false)
provide(settingsPageKey, { title: () => props.title, nested })

const bar = inject(settingsLevelKey, null)
/** The level this page put on the dialog's bar, which only it takes off again. */
let level: SettingsLevel | null = null
function leaveBar(): void {
  if (bar && level && bar.value === level) {
    bar.value = null
  }
  level = null
}
watch(
  () => props.back,
  (back) => {
    leaveBar()
    if (bar && back) {
      level = { label: back, back: () => emit('back') }
      bar.value = level
    }
  },
  { immediate: true },
)
onBeforeUnmount(leaveBar)
</script>

<template>
  <div
    class="mx-auto flex w-full flex-col gap-8"
    :class="[wide ? 'max-w-none' : 'max-w-2xl', fill ? '@md:h-full @md:min-h-0' : '']"
  >
    <header class="select-none" :class="nested ? 'hidden @md:block' : ''">
      <!-- In the dialog, its narrow bar goes back; elsewhere, and wide, this button does. -->
      <div v-if="back" class="-ml-2 mb-2" :class="bar ? 'hidden @md:block' : ''">
        <Button variant="ghost" size="sm" @click="emit('back')">
          <ChevronLeft :size="ICON_PX.in24" />
          {{ back }}
        </Button>
      </div>
      <h2 class="text-[20px] font-medium leading-7 text-fg-emphasis">{{ title }}</h2>
      <p v-if="description" class="mt-1 text-[13px] leading-5 text-fg-muted">{{ description }}</p>
    </header>
    <slot />
  </div>
</template>
