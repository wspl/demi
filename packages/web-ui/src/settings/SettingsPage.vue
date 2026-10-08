<script setup lang="ts">
import { inject, onBeforeUnmount, provide, ref, watch } from 'vue'
import { ChevronLeft } from '@lucide/vue'
import Button from '../ui/Button.vue'
import TruncatedText from '../ui/TruncatedText.vue'
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
 *
 * A page of one thing says its state in its header, never in rows: an
 * `icon` beside the title, a `status` line under it (a dot and a few
 * words), the `description` sentence that explains it, and `actions` on a
 * line of their own under the text, starting where it starts, as macOS's
 * Network settings head a service's page. A row below is only a setting or
 * a fact.
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
      <div class="flex flex-wrap items-start gap-x-4 gap-y-3">
        <div v-if="$slots.icon" class="flex size-10 shrink-0 items-center justify-center rounded-lg bg-surface-float text-fg-muted ring-1 ring-line">
          <slot name="icon" />
        </div>
        <div class="min-w-60 flex-1">
          <h2 class="text-[20px] font-medium leading-7 text-fg-emphasis">
            <TruncatedText :text="title" />
          </h2>
          <div v-if="$slots.status" class="mt-0.5 flex items-center gap-1.5 text-[13px] leading-5 text-fg-body">
            <slot name="status" />
          </div>
          <p v-if="description || $slots.description" class="mt-1 text-[13px] leading-5 text-fg-muted">
            <slot name="description">{{ description }}</slot>
          </p>
          <!-- The header's buttons: a line of their own, in every state, so they never move with the text. -->
          <div v-if="$slots.actions" class="mt-3 flex flex-wrap items-center gap-2">
            <slot name="actions" />
          </div>
        </div>
      </div>
    </header>
    <slot />
  </div>
</template>
