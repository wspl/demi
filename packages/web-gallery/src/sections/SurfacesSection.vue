<script setup lang="ts">
import GallerySection from '../components/GallerySection.vue'

const swatches = [
  { name: 'base', bg: 'var(--surface-base)', fg: 'var(--fg)' },
  { name: 'surface', bg: 'var(--surface)', fg: 'var(--fg)' },
  { name: 'editor', bg: 'var(--surface-editor)', fg: 'var(--fg)' },
  { name: 'raised', bg: 'var(--surface-raised)', fg: 'var(--fg)' },
  { name: 'card', bg: 'var(--surface-card)', fg: 'var(--fg)' },
  { name: 'float', bg: 'var(--surface-float)', fg: 'var(--fg)' },
]

const fillSurfaces = [
  { name: 'base', class: 'bg-surface-base' },
  { name: 'surface', class: 'bg-surface' },
  { name: 'editor', class: 'bg-surface-editor' },
  { name: 'raised', class: 'bg-surface-raised' },
  { name: 'card', class: 'bg-surface-card' },
  { name: 'float', class: 'bg-surface-float' },
]

const fills = [
  { name: 'hover', class: 'bg-hover' },
  { name: 'active', class: 'bg-active' },
  { name: 'button', class: 'bg-btn' },
  { name: 'accent', class: 'bg-tint-accent' },
]

// A bordered control at rest, hovered and pressed, each state pinned with the utility the
// control's own :hover and pressed rules apply (base.css), and the stronger edge they set.
const controlStates = [
  { name: 'rest', fill: '', edge: '' },
  { name: 'hover', fill: 'bg-hover', edge: 'shadow-[var(--shadow-btn-hover)]' },
  { name: 'pressed', fill: 'bg-active', edge: 'shadow-[var(--shadow-btn-hover)]' },
]

const borderedControls = [
  { name: 'Button', class: 'btn h-7 rounded-md text-chrome font-medium text-fg-body', edged: true },
  { name: 'Chip', class: 'btn-solid h-7 rounded-full text-chrome text-fg-body', edged: true },
  { name: 'Pill', class: 'file-pill btn h-[22px] self-center rounded-full text-xs text-fg-muted [--shadow-btn:var(--shadow-pill)]', edged: true },
  // A choice card rests on the float surface wherever it sits, and steps from it.
  { name: 'Card', class: 'h-9 rounded-lg border text-chrome text-fg-emphasis [--rest-fill:var(--surface-float)]', edged: false },
]

/** A pinned state's classes for a control: its fill, and its stronger edge. */
function stateClass(control: (typeof borderedControls)[number], state: (typeof controlStates)[number]): string[] {
  if (control.edged) {
    return [state.fill, state.edge]
  }
  return state.fill ? [state.fill, 'border-line-strong'] : ['bg-surface-float', 'border-line']
}

const accentStates = [
  { name: 'rest', fill: '' },
  { name: 'hover', fill: 'bg-accent-hover' },
  { name: 'pressed', fill: 'bg-accent-active' },
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
    <GallerySection
      title="Surfaces"
      note="Six fills. Float is for menus, tooltips and dialogs. Card is what stands off the reading surface in flow, a message bubble or a notice over the composer: a step lighter in dark, as raised is, and a step darker in light, where the reading surface is white."
    >
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

    <GallerySection
      title="Control Fills"
      note="Control fills are opaque: each is the surface the control sits on mixed with the fill’s tint, so nothing under a control shows through it, and a control on a selected row takes the row’s fill as its surface. Translucency is for scrims, shadows and edges only. Hover and pressed show in the fill: a control drawn with an edge (a button, a chip, a file pill, a choice card) takes its own resting fill one hover step on, and one step further when pressed, so its hover reads on every surface; its edge strengthens with it but never shows hover alone. The accent’s default button takes the same steps in lightness."
    >
      <div class="grid gap-3 md:grid-cols-5">
        <div
          v-for="surface in fillSurfaces"
          :key="surface.name"
          :class="surface.class"
          class="flex flex-col gap-1.5 rounded-xl border border-line p-3"
        >
          <span class="gallery-label pb-1">{{ surface.name }}</span>
          <span
            v-for="fill in fills"
            :key="fill.name"
            :class="fill.class"
            class="flex h-7 items-center rounded-md px-2 text-chrome text-fg-body"
          >{{ fill.name }}</span>
          <div class="grid grid-cols-3 gap-1.5 pt-3" :data-control-states="surface.name">
            <span
              v-for="state in controlStates"
              :key="state.name"
              class="gallery-label"
            >{{ state.name }}</span>
            <template v-for="control in borderedControls" :key="control.name">
              <span
                v-for="state in controlStates"
                :key="state.name"
                :class="[control.class, stateClass(control, state)]"
                :data-control="control.name"
                :data-state="state.name"
                class="flex min-w-0 items-center justify-center px-1"
              >{{ control.name }}</span>
            </template>
            <span
              v-for="state in accentStates"
              :key="state.name"
              :class="state.fill"
              data-control="Primary"
              :data-state="state.name"
              class="btn-primary flex h-7 min-w-0 items-center justify-center rounded-md px-1 text-chrome font-medium text-white"
            >Primary</span>
          </div>
        </div>
      </div>
    </GallerySection>

    <GallerySection title="Type Ramp" note="Emphasis through ghost.">
      <div class="gallery-frame divide-y divide-line">
        <div
          v-for="item in textSteps"
          :key="item.name"
          class="flex items-center justify-between px-4 py-3"
        >
          <span class="text-[13px]" :style="{ color: item.color }">The login test still expects the old cookie name.</span>
          <span class="gallery-label">{{ item.name }}</span>
        </div>
      </div>
    </GallerySection>

    <GallerySection title="Lines" note="Subtle through overlay.">
      <div class="space-y-3">
        <div
          v-for="item in lines"
          :key="item.name"
          class="space-y-1"
        >
          <div class="gallery-label">{{ item.name }}</div>
          <div class="h-px" :style="{ background: item.color }" />
        </div>
      </div>
    </GallerySection>
  </div>
</template>
