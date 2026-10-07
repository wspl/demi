<script setup lang="ts">
/** A rule for UI text, with what follows it, what breaks it, and where it comes from. */
export interface TextRule {
  title: string
  rule: string
  right: readonly string[]
  wrong: readonly string[]
  /** The products or guides that do it this way. */
  source: string
}

/** The Writing page's rules as cards: the rule, its examples, its source. */
defineProps<{
  rules: readonly TextRule[]
}>()
</script>

<template>
  <div class="grid max-w-5xl gap-3 md:grid-cols-2">
    <div v-for="item in rules" :key="item.title" class="space-y-2 rounded-xl border border-line p-3">
      <h3 class="text-[13px] font-medium text-fg-emphasis">{{ item.title }}</h3>
      <p class="text-[12px] leading-5 text-fg-body">{{ item.rule }}</p>
      <div class="flex flex-wrap gap-1 text-[12px] leading-4">
        <span v-for="example in item.right" :key="example" class="rounded bg-overlay/6 px-1.5 py-0.5 text-fg-body">{{ example }}</span>
        <span v-for="example in item.wrong" :key="example" class="rounded px-1.5 py-0.5 text-fg-muted line-through">{{ example }}</span>
      </div>
      <p class="text-[11px] leading-4 text-fg-subtle">{{ item.source }}</p>
    </div>
  </div>
</template>
