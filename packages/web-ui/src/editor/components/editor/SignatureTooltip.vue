<script setup lang="ts">
import { computed, ref } from 'vue'
import { ChevronDown, ChevronUp } from '@lucide/vue'

interface Signature {
  label: string
  documentation?: string
  parameters?: Array<{ label: string | [number, number]; documentation?: string }>
  activeParameter?: number
}

const props = defineProps<{
  signatures: Signature[]
  activeSignature?: number
  activeParameter?: number
}>()

const currentIndex = ref(props.activeSignature ?? 0)

const sig = computed(() => props.signatures[currentIndex.value])
const activeParamIdx = computed(() => sig.value?.activeParameter ?? props.activeParameter ?? 0)
const param = computed(() => sig.value?.parameters?.[activeParamIdx.value])

const labelParts = computed(() => {
  if (!sig.value) return []
  const p = param.value
  if (p && Array.isArray(p.label)) {
    const [from, to] = p.label
    return [
      { text: sig.value.label.slice(0, from), active: false },
      { text: sig.value.label.slice(from, to), active: true },
      { text: sig.value.label.slice(to), active: false },
    ].filter((part) => part.text)
  }
  return [{ text: sig.value.label, active: false }]
})

function prev() {
  if (currentIndex.value > 0) currentIndex.value--
}

function next() {
  if (currentIndex.value < props.signatures.length - 1) currentIndex.value++
}
</script>

<template>
  <div v-if="sig" class="max-w-[600px] p-2 text-xs leading-relaxed">
    <div v-if="signatures.length > 1" class="mb-1 flex items-center gap-1 text-fg-subtle">
      <ChevronUp :size="18" class="cursor-pointer rounded text-fg-muted hover:bg-active" @click="prev" />
      <span>{{ currentIndex + 1 }}/{{ signatures.length }}</span>
      <ChevronDown :size="18" class="cursor-pointer rounded text-fg-muted hover:bg-active" @click="next" />
    </div>
    <code class="whitespace-pre-wrap text-fg">
      <template v-for="(part, i) in labelParts" :key="i">
        <span v-if="part.active" class="font-bold text-on-accent">{{ part.text }}</span>
        <span v-else>{{ part.text }}</span>
      </template>
    </code>
    <div v-if="param?.documentation" class="mt-1.5 border-t border-fg-ghost pt-1.5 text-fg-muted">
      <span class="text-fg-subtle">@param</span> {{ param.documentation }}
    </div>
    <div v-if="sig.documentation" class="mt-1.5 border-t border-fg-ghost pt-1.5 text-fg-muted">{{ sig.documentation }}</div>
  </div>
</template>
