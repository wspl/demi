<script setup lang="ts">
import { computed } from 'vue'
import { RouterLink, RouterView, useRoute } from 'vue-router'
import ToastHost from '@demicodes/web-ui/ui/ToastHost.vue'
import { applyParadigm, galleryState, PARADIGMS } from './gallery-state'
import { NAV } from './router'
import AccentPicker from './components/AccentPicker.vue'
import AxisPicker from './components/AxisPicker.vue'

const route = useRoute()

function setMode(mode: 'light' | 'dark') {
  galleryState.mode = mode
}

const mainClass = computed(() => {
  if (route.meta.layout === 'preview') return 'flex min-h-0 flex-1 flex-col overflow-hidden'
  if (route.meta.layout === 'session') return 'min-h-0 flex-1 overflow-y-auto px-5 py-4'
  return 'min-h-0 flex-1 overflow-y-auto px-6 py-6'
})
</script>

<template>
  <div class="flex h-full bg-surface-base text-fg">
    <aside class="select-none flex w-56 shrink-0 flex-col border-r border-line bg-surface">
      <div class="px-4 py-4">
        <div class="text-[13px] font-medium text-fg-emphasis">Demi Gallery</div>
        <div class="mt-1 text-[12px] leading-4 text-fg-subtle">@demicodes/web-ui</div>
      </div>
      <nav class="flex flex-1 flex-col gap-0.5 px-2">
        <RouterLink
          v-for="item in NAV"
          :key="item.path"
          :to="item.path"
          class="rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors duration-200 ease-out"
          :class="route.path === item.path
            ? 'bg-active text-fg-emphasis'
            : 'text-fg-muted hover:bg-hover hover:text-fg'"
        >
          {{ item.label }}
        </RouterLink>
      </nav>
    </aside>

    <div class="flex min-w-0 flex-1 flex-col">
      <header class="select-none shrink-0 border-b border-line bg-surface px-5 py-3">
        <div class="flex flex-wrap items-start gap-6">
          <div class="flex min-w-[220px] flex-1 flex-col gap-1.5">
            <div class="gallery-label">Paradigm</div>
            <div class="flex flex-wrap gap-1">
              <button
                v-for="item in PARADIGMS"
                :key="item.id"
                type="button"
                class="rounded-md px-2 py-1 text-[12px] transition-colors duration-200 ease-out"
                :class="galleryState.paradigm === item.id
                  ? 'bg-active text-fg-emphasis'
                  : 'text-fg-muted hover:bg-hover hover:text-fg'"
                @click="applyParadigm(item.id)"
              >
                {{ item.name }}
              </button>
              <span
                v-if="galleryState.paradigm === 'custom'"
                class="rounded-md bg-hover px-2 py-1 text-[12px] text-fg-muted"
              >Custom</span>
            </div>
          </div>
          <AxisPicker
            label="Mode"
            :values="(['dark', 'light'] as const)"
            :model-value="galleryState.mode"
            @update:model-value="setMode"
          />
          <AxisPicker
            label="Tone"
            :values="(['zinc', 'cool', 'warm', 'ink'] as const)"
            :model-value="galleryState.tone"
            @update:model-value="galleryState.tone = $event"
          />
          <AccentPicker
            :model-value="galleryState.accent"
            @update:model-value="galleryState.accent = $event"
          />
          <AxisPicker
            label="Density"
            :values="(['compact', 'regular', 'comfortable'] as const)"
            :model-value="galleryState.density"
            @update:model-value="galleryState.density = $event"
          />
          <AxisPicker
            label="Radius"
            :values="(['tight', 'medium', 'soft'] as const)"
            :model-value="galleryState.radius"
            @update:model-value="galleryState.radius = $event"
          />
          <AxisPicker
            label="Shadow"
            :values="(['hairline', 'soft', 'carved'] as const)"
            :model-value="galleryState.shadow"
            @update:model-value="galleryState.shadow = $event"
          />
        </div>
      </header>

      <main :class="mainClass">
        <RouterView />
      </main>
    </div>
    <ToastHost />
  </div>
</template>
