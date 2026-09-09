<script setup lang="ts">
import { ref } from 'vue'
import { Search } from '@lucide/vue'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import CopyCode from '@demicodes/web-ui/ui/CopyCode.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'

const sizes = ['sm', 'md', 'lg'] as const
const widths = [240, 400] as const
const variants = ['plain', 'prefix', 'suffix', 'secret', 'disabled', 'bare'] as const
const value = ref('Example value')
const selected = ref('first')
const segmentOptions = [
  { value: 'first', label: 'First' },
  { value: 'second', label: 'Second' },
  { value: 'third', label: 'Third' },
]
const surfaces = [
  { name: 'base', class: 'bg-surface-base' },
  { name: 'surface', class: 'bg-surface' },
  { name: 'raised', class: 'bg-surface-raised' },
  { name: 'float', class: 'bg-surface-float' },
]

</script>

<template>
  <GallerySection
    title="Control layout"
    note="Production CSS checks: all input sizes, fixed and flexible widths, adornments, disabled and bare fields. Command text and copy controls share a vertical center."
  >
    <div class="mb-6 flex flex-wrap gap-4">
      <div
        v-for="surface in surfaces"
        :key="surface.name"
        :class="surface.class"
        :data-segment-surface="surface.name"
        class="flex flex-col items-start gap-3 rounded-xl border border-line p-4"
      >
        <p class="text-chrome text-fg-muted">{{ surface.name }}</p>
        <Segmented v-model="selected" :options="segmentOptions" size="sm" />
        <Segmented v-model="selected" :options="segmentOptions" />
        <Segmented v-model="selected" :options="segmentOptions" disabled />
      </div>
    </div>
    <div class="flex flex-wrap items-start gap-6">
      <GallerySpecimen
        v-for="width in widths"
        :key="width"
        :variant="`${width}px container`"
      >
        <div
          class="flex max-w-full flex-col gap-4"
          :style="{ width: `${width}px` }"
        >
          <div
            v-for="size in sizes"
            :key="size"
            class="flex flex-col gap-2"
          >
            <p class="text-chrome text-fg-muted">{{ size }}</p>
            <div
              v-for="variant in variants"
              :key="variant"
              class="flex min-w-0 items-center gap-2"
              :data-input-layout="`${width}-${size}-${variant}`"
              :data-size="size"
              :data-variant="variant"
            >
              <TextInput
                v-model="value"
                :size="size"
                :aria-label="`${width} ${size} ${variant}`"
                :secret="variant === 'secret'"
                :disabled="variant === 'disabled'"
                :bare="variant === 'bare'"
                class="flex-1"
              >
                <template
                  v-if="variant === 'prefix'"
                  #prefix
                  ><Search :size="14"
                /></template>
                <template
                  v-if="variant === 'suffix'"
                  #suffix
                  ><span class="text-chrome">K</span></template
                >
              </TextInput>
              <Button
                :size="size"
                :disabled="variant === 'disabled'"
                >Action</Button
              >
            </div>
          </div>
          <div data-fixed-input>
            <TextInput
              v-model="value"
              class="w-32 max-w-full"
              aria-label="Fixed width"
            />
          </div>
          <div data-command-layout="single">
            <CopyCode code="claude setup-token" />
          </div>
          <div data-command-layout="wrapped">
            <CopyCode
              code="demi-runner run --backend https://backend.example.test/acceptance"
            />
          </div>
        </div>
      </GallerySpecimen>
    </div>
  </GallerySection>
</template>
