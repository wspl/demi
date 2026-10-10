<script setup lang="ts">
import { computed, shallowRef, watch } from 'vue'
import { File, Folder } from '@lucide/vue'
import { ensureFileIconTheme, fileIconTheme, fileIconUrl } from './file-icon-theme'
import { fileIconGlyph, fileIconName } from './file-icons'

/**
 * A file or folder glyph from the Material Icon Theme, chosen by name: `src`
 * gets the source folder, `package.json` the Node badge, `App.vue` the Vue mark.
 * Until the theme has loaded, and for a name it does not know, a plain outline.
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

ensureFileIconTheme()

/** Where the glyph is in the theme's package, once the theme has loaded. */
const glyph = computed(() => {
  const theme = fileIconTheme.value
  return theme
    ? fileIconGlyph(theme, props.icon ?? fileIconName(theme, props.name, props.isDirectory))
    : null
})
const src = shallowRef<string | null>(null)

watch(
  glyph,
  (path) => {
    src.value = null
    if (!path)
      return
    fileIconUrl(path).then((url) => {
      if (glyph.value === path)
        src.value = url
    })
  },
  { immediate: true },
)
</script>

<template>
  <img
    v-if="src"
    :src="src"
    :width="size"
    :height="size"
    class="max-w-none shrink-0"
    alt=""
    draggable="false"
  />
  <component
    :is="isDirectory ? Folder : File"
    v-else
    :size="size - 2"
    class="shrink-0"
  />
</template>
