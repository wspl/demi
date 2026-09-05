<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import type { Finding, FindingSeverity } from '../color-review/findings'
import type { ReviewChoice, ReviewDecision } from '../color-review/decisions'

const props = defineProps<{
  finding: Finding
  decision: ReviewDecision | null
  index: number
}>()

const emit = defineEmits<{
  decide: [choice: ReviewChoice, note: string]
}>()

const SEVERITY: Record<FindingSeverity, { label: string; className: string }> = {
  high: { label: '严重', className: 'bg-tint-danger text-on-danger' },
  medium: { label: '中等', className: 'bg-tint-warning text-on-warning' },
  low: { label: '轻微', className: 'bg-hover text-fg-muted' },
}

const CHOICES: { id: ReviewChoice; label: string }[] = [
  { id: 'accept', label: '可以' },
  { id: 'keep', label: '不用改' },
  { id: 'instruct', label: '需要按我的指示改' },
]

const note = ref(props.decision?.note ?? '')
watch(() => props.decision?.note, (value) => {
  if (value != null && value !== note.value) note.value = value
})

const choice = computed(() => props.decision?.choice ?? null)
const noteDirty = computed(() => choice.value === 'instruct' && note.value !== (props.decision?.note ?? ''))

function pick(next: ReviewChoice): void {
  emit('decide', next, next === 'instruct' ? note.value : '')
}

function submitNote(): void {
  emit('decide', 'instruct', note.value)
}

const savedAt = computed(() => {
  const stamp = props.decision?.updatedAt
  if (!stamp) return ''
  const date = new Date(stamp)
  return `${date.getHours().toString().padStart(2, '0')}:${date.getMinutes().toString().padStart(2, '0')}`
})
</script>

<template>
  <section :id="finding.id" class="gallery-frame overflow-hidden">
    <header class="flex flex-wrap items-center gap-2 border-b border-line px-4 py-3">
      <span class="font-mono text-[11px] text-fg-faint">{{ String(index + 1).padStart(2, '0') }}</span>
      <h3 class="text-[14px] font-medium text-fg-emphasis">{{ finding.title }}</h3>
      <span class="rounded px-1.5 py-0.5 text-[11px]" :class="SEVERITY[finding.severity].className">
        {{ SEVERITY[finding.severity].label }}
      </span>
      <span class="ml-auto text-[12px] text-fg-subtle">{{ finding.where }}</span>
    </header>

    <div class="grid gap-x-6 gap-y-4 px-4 py-4 text-[13px] leading-5 md:grid-cols-[1fr_1fr]">
      <div class="space-y-3">
        <div>
          <div class="gallery-label mb-1">问题</div>
          <p class="text-fg-body">{{ finding.problem }}</p>
        </div>
        <div>
          <div class="gallery-label mb-1">依据</div>
          <ul class="list-disc space-y-0.5 pl-5 text-fg-muted">
            <li v-for="line in finding.evidence" :key="line">{{ line }}</li>
          </ul>
        </div>
      </div>
      <div class="space-y-3">
        <div>
          <div class="gallery-label mb-1">方案</div>
          <ul class="list-disc space-y-0.5 pl-5 text-fg-body">
            <li v-for="line in finding.proposal" :key="line">{{ line }}</li>
          </ul>
        </div>
        <div>
          <div class="gallery-label mb-1">改动文件</div>
          <ul class="space-y-0.5 font-mono text-[11px] text-fg-subtle">
            <li v-for="file in finding.files" :key="file">{{ file }}</li>
          </ul>
        </div>
      </div>
    </div>

    <div class="grid border-t border-line md:grid-cols-2 md:divide-x md:divide-line">
      <div class="min-w-0 px-4 py-3">
        <div class="gallery-label mb-2">现状</div>
        <div class="cr-stage">
          <slot />
        </div>
      </div>
      <div class="min-w-0 px-4 py-3">
        <div class="gallery-label mb-2">修改后</div>
        <div class="cr-stage cr-after" :data-fix="finding.id">
          <slot />
        </div>
      </div>
    </div>

    <footer class="border-t border-line bg-surface-base/40 px-4 py-3">
      <div class="flex flex-wrap items-center gap-2">
        <span class="text-[12px] text-fg-subtle">你的决定</span>
        <Button
          v-for="item in CHOICES"
          :key="item.id"
          size="sm"
          :variant="choice === item.id ? 'primary' : 'default'"
          @click="pick(item.id)"
        >
          {{ item.label }}
        </Button>
        <span v-if="savedAt" class="ml-auto text-[11px] text-fg-faint">已记录 {{ savedAt }}</span>
      </div>
      <div v-if="choice === 'instruct'" class="mt-3 flex flex-col gap-2">
        <textarea
          v-model="note"
          rows="3"
          placeholder="写下你希望怎么改……"
          class="w-full resize-y rounded-md border border-line bg-surface-editor px-2.5 py-2 text-[13px] leading-5 text-fg-body outline-none placeholder:text-fg-faint focus:border-line-focus"
        />
        <div class="flex items-center gap-2">
          <Button size="sm" variant="primary" :disabled="!noteDirty" @click="submitNote">记录指示</Button>
          <span v-if="noteDirty" class="text-[11px] text-on-warning">尚未记录</span>
        </div>
      </div>
    </footer>
  </section>
</template>
