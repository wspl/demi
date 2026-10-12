<script setup lang="ts">
import { computed, ref } from 'vue'
import { useFitMore } from './fit-more'
import Tag from './Tag.vue'
import Tooltip from './Tooltip.vue'

/** Tags on one line. What does not fit collapses to +N. */
const props = defineProps<{
  items: string[]
}>()

/** The space between tags, `gap-1`. */
const GAP_PX = 4

const root = ref<HTMLElement | null>(null)
// An invisible copy of every tag and of the count, which gives the widths the line is fitted by.
const meter = ref<HTMLElement | null>(null)

const shown = useFitMore({
  items: () => props.items,
  box: () => root.value,
  measure: () => {
    const strip = meter.value
    if (!root.value || !strip)
      return null
    const more = strip.querySelector<HTMLElement>('[data-more]')
    return {
      room: root.value.clientWidth,
      widths: [...strip.querySelectorAll<HTMLElement>('[data-chip]')].map((chip) => chip.offsetWidth),
      gap: GAP_PX,
      countWidth: () => more?.offsetWidth ?? 28,
    }
  },
})
const hidden = computed(() => props.items.slice(shown.value))
</script>

<template>
  <div
    v-if="items.length"
    ref="root"
    class="relative min-w-0"
  >
    <div
      ref="meter"
      class="pointer-events-none invisible absolute inset-x-0 top-0 flex flex-nowrap gap-1"
      aria-hidden="true"
    >
      <span
        v-for="item in items"
        :key="item"
        data-chip
        class="inline-flex"
      >
        <Tag>{{ item }}</Tag>
      </span>
      <span data-more class="inline-flex"><Tag>+99</Tag></span>
    </div>
    <div
      class="flex h-[18px] min-w-0 flex-nowrap items-center gap-1 overflow-hidden leading-none"
    >
      <Tag v-for="item in items.slice(0, shown)" :key="item">{{ item }}</Tag>
      <Tooltip v-if="hidden.length">
        <Tag>+{{ hidden.length }}</Tag>
        <template #overlay>
          <div class="flex flex-col gap-0.5">
            <div v-for="item in hidden" :key="item">{{ item }}</div>
          </div>
        </template>
      </Tooltip>
    </div>
  </div>
</template>
