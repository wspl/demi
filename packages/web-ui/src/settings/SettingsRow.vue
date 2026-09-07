<script setup lang="ts">
/**
 * Icons align with the title. Controls center unless detail is present;
 * controlsAlign explicitly selects title alignment or vertical centering.
 * Label and explanation on the left, the control on the right. In a narrow card the
 * control drops under the text and keeps its right alignment. An inset row belongs to
 * the row above it (a server's tools, an agent's permissions). A compact row is for
 * lists of like items (models, accounts); an interactive one opens on click.
 */
defineProps<{
  label: string
  description?: string
  inset?: boolean
  compact?: boolean
  interactive?: boolean
  isolateControls?: boolean
  controlsAlign?: 'start' | 'center'
}>()

const emit = defineEmits<{
  click: []
}>()
</script>

<template>
  <div
    class="flex items-center gap-y-2 @sm:flex-nowrap"
    :class="[
      inset ? 'min-h-9 flex-nowrap gap-x-3 bg-overlay/[0.025] py-1.5 pl-7 pr-4' : compact ? 'min-h-10 flex-wrap gap-x-3 px-3 py-1.5' : 'min-h-14 flex-wrap gap-x-4 px-4 py-3',
      interactive ? 'cursor-default transition-colors duration-200 ease-out hover:bg-overlay/[0.03]' : '',
    ]"
    @click="interactive && emit('click')"
  >
    <div v-if="$slots.leading" class="flex h-5 shrink-0 items-center text-fg-muted" :style="{ alignSelf: description || $slots.description || $slots.detail ? 'flex-start' : 'center' }">
      <slot name="leading" />
    </div>
    <div class="min-w-0 flex-1 select-none">
      <div class="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1" :class="inset ? 'text-[12px] text-fg-body' : 'text-chrome text-fg'">
        <span class="max-w-full truncate">{{ label }}</span>
        <div v-if="$slots.tags" class="flex h-5 shrink-0 items-center gap-1">
          <slot name="tags" />
        </div>
      </div>
      <div v-if="description || $slots.description" class="mt-0.5 text-[12px] leading-4 text-fg-subtle">
        <slot name="description">{{ description }}</slot>
      </div>
    </div>
    <!-- Beside the text the controls may take up to two thirds; inputs shrink, buttons never wrap.
         The container spans the row's content box so a bare input can stretch to it. -->
    <div
      v-if="$slots.default"
      @click="isolateControls && $event.stopPropagation()"
      class="flex min-w-0 items-center justify-end gap-2 self-stretch @sm:basis-auto @sm:max-w-[66%]"
      :class="[
        inset ? 'shrink-0' : 'basis-full',
        (controlsAlign ?? ($slots.detail ? 'start' : 'center')) === 'start' ? '@sm:h-5 @sm:self-start' : '',
      ]"
    >
      <slot />
    </div>
  </div>
</template>
