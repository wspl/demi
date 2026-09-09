<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Tag from './Tag.vue'
import Tooltip from './Tooltip.vue'

/** Tags on one line. What does not fit collapses to +N. */
const props = defineProps<{
  items: string[]
}>()

const root = ref<HTMLElement | null>(null)
const meter = ref<HTMLElement | null>(null)
const shown = ref(props.items.length)
const hidden = computed(() => props.items.slice(shown.value))

let observer: ResizeObserver | undefined

function fit() {
  const box = root.value
  const strip = meter.value
  if (!box || !strip || props.items.length === 0) {
    shown.value = props.items.length
    return
  }
  const max = box.clientWidth
  const chips = [...strip.querySelectorAll<HTMLElement>('[data-chip]')]
  const more = strip.querySelector<HTMLElement>('[data-more]')
  const gap = 4
  const moreWidth = more?.offsetWidth ?? 28
  const widths = chips.map((chip) => chip.offsetWidth)
  const full = widths.reduce((sum, width, index) => sum + width + (index
    ? gap
    : 0), 0)
  if (full <= max) {
    shown.value = widths.length
    return
  }
  let used = 0
  let count = 0
  for (let i = 0; i < widths.length; i++) {
    const next = used + (count ? gap : 0) + widths[i]!
    const rest = widths.length - i - 1
    if (next + (rest > 0 ? gap + moreWidth : 0) > max)
      break
    used = next
    count++
  }
  shown.value = count
}

onMounted(() => {
  observer = new ResizeObserver(() => {
    void nextTick(fit)
  })
  if (root.value)
    observer.observe(root.value)
  void nextTick(fit)
})

watch(() => props.items, () => {
  shown.value = props.items.length
  void nextTick(fit)
})

onBeforeUnmount(() => observer?.disconnect())
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
