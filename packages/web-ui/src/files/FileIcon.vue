<script setup lang="ts">
import { computed } from 'vue'
import { File, Folder } from '@lucide/vue'
import { regionForm } from '../infra/region-wait'
import { ensureFileIconTheme, fileIconTheme, iconPicture } from './file-icon-theme'
import { fileIconGlyph, fileIconName } from './file-icons'

/**
 * A file or folder glyph from the Material Icon Theme, chosen by name: `src` gets the
 * source folder, `package.json` the Node badge, `App.vue` the Vue mark.
 *
 * The box has its size from the start, whatever it holds. The icon is a region
 * (`infra/region-wait.ts`) that needs the theme's list and then the glyph's
 * picture: for 300 ms at most, in all, the box holds nothing; past that, a plain
 * outline, as for a name the theme does not know, which stays until the picture
 * replaces it.
 */
const props = withDefaults(
  defineProps<{
    name: string
    isDirectory: boolean
    /** A theme icon id that overrides what the name resolves to. */
    icon?: string
    size?: number
  }>(),
  { size: 16 },
)

void ensureFileIconTheme()

/** Where the glyph is in the theme's package, once the theme has loaded. */
const glyph = computed(() => {
  const theme = fileIconTheme.value
  return theme
    ? fileIconGlyph(theme, props.icon ?? fileIconName(theme, props.name, props.isDirectory))
    : null
})
/** The glyph's picture: undefined while it is on its way, null where there is none. */
const src = computed(() => (glyph.value === null ? null : iconPicture(glyph.value)))

const form = computed(() => regionForm(
  `icon:${props.isDirectory ? 'folder' : 'file'}:${props.icon ?? props.name}`,
  () => fileIconTheme.value !== null && (glyph.value === null || src.value !== undefined),
))
</script>

<template>
  <!-- One box, so a row's width and height do not change when the list or the picture arrives. -->
  <span
    class="inline-flex shrink-0 items-center justify-center"
    :style="{ width: `${size}px`, height: `${size}px` }"
  >
    <img
      v-if="src && form === 'final'"
      :src="src"
      :width="size"
      :height="size"
      class="max-w-none shrink-0"
      alt=""
      aria-hidden="true"
      draggable="false"
    />
    <component :is="isDirectory ? Folder : File" v-else-if="form === 'plain' || (form === 'final' && !src)" :size="size - 2" />
  </span>
</template>
