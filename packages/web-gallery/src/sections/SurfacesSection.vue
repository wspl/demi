<script setup lang="ts">
import GallerySection from '../components/GallerySection.vue'

const swatches = [
  { name: 'base', bg: 'var(--surface-base)', fg: 'var(--fg)' },
  { name: 'surface', bg: 'var(--surface)', fg: 'var(--fg)' },
  { name: 'editor', bg: 'var(--surface-editor)', fg: 'var(--fg)' },
  { name: 'raised', bg: 'var(--surface-raised)', fg: 'var(--fg)' },
  { name: 'float', bg: 'var(--surface-float)', fg: 'var(--fg)' },
]

const textSteps = [
  { name: 'emphasis', color: 'var(--fg-emphasis)' },
  { name: 'fg', color: 'var(--fg)' },
  { name: 'body', color: 'var(--fg-body)' },
  { name: 'muted', color: 'var(--fg-muted)' },
  { name: 'subtle', color: 'var(--fg-subtle)' },
  { name: 'faint', color: 'var(--fg-faint)' },
  { name: 'ghost', color: 'var(--fg-ghost)' },
]

const lines = [
  { name: 'subtle', color: 'var(--line-subtle)' },
  { name: 'line', color: 'var(--line)' },
  { name: 'strong', color: 'var(--line-strong)' },
  { name: 'focus', color: 'var(--line-focus)' },
  { name: 'overlay', color: 'var(--line-overlay)' },
]
</script>

<template>
  <div class="space-y-8">
    <GallerySection title="Surfaces" note="Five fills. float is for menus, tooltips, and dialogs.">
      <div class="grid gap-3 md:grid-cols-5">
        <div
          v-for="item in swatches"
          :key="item.name"
          class="flex h-28 flex-col justify-between rounded-xl p-3"
          :style="{ background: item.bg, color: item.fg, boxShadow: 'var(--shadow-md)' }"
        >
          <span class="gallery-label">{{ item.name }}</span>
          <span class="text-[12px] break-all opacity-70">{{ item.bg }}</span>
        </div>
      </div>
    </GallerySection>

    <GallerySection title="Type ramp" note="Emphasis through ghost.">
      <div class="gallery-frame divide-y divide-line">
        <div v-for="item in textSteps" :key="item.name" class="flex items-center justify-between px-4 py-3">
          <span class="text-[13px]" :style="{ color: item.color }">The login test still expects the old cookie name.</span>
          <span class="gallery-label">{{ item.name }}</span>
        </div>
      </div>
    </GallerySection>

    <GallerySection title="Lines" note="Subtle through overlay.">
      <div class="space-y-3">
        <div v-for="item in lines" :key="item.name" class="space-y-1">
          <div class="gallery-label">{{ item.name }}</div>
          <div class="h-px" :style="{ background: item.color }" />
        </div>
      </div>
    </GallerySection>
  </div>
</template>
