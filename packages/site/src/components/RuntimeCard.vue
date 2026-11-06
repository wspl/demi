<script setup lang="ts">
// Demi's runtime as a card at the browser's corner, beside a demo whose
// scene says what it uses: Demi, with the marks of the models it runs on,
// its one tool, the shell, and what the shell runs, the system's programs and the `demi` commands for the browser,
// other machines and other agents. The part the agent uses now lights up,
// and a bar shows how full the context the model reads is, which a
// compaction empties.
import { computed } from 'vue'
import claudeUrl from '@demicodes/web-ui/settings/logos/claude.svg?url&no-inline'
import geminiUrl from '@demicodes/web-ui/settings/logos/google.svg?url&no-inline'
import openaiUrl from '@demicodes/web-ui/settings/logos/openai.svg?url&no-inline'
import { OTTER_SVG } from '@/brand'
import type { RuntimePart, StageCard } from '@/demo/message'
import { useKept } from './stageCard'

type Runtime = Extract<StageCard, { kind: 'runtime' }>

const props = defineProps<{ card: Runtime | null }>()

const last = useKept(() => props.card, { kind: 'runtime', part: null, context: 0 })

/**
 * The models' makers in their brand colors, each mark at the share of its box
 * that makes the three look one size, as the models' screen sizes them.
 */
const MAKERS: readonly { name: string; url: string; fit: number; paint: string }[] = [
  { name: 'Claude', url: claudeUrl, fit: 0.92, paint: '#d97757' },
  { name: 'ChatGPT', url: openaiUrl, fit: 1.28, paint: '#0d0d0d' },
  { name: 'Gemini', url: geminiUrl, fit: 1.1, paint: 'linear-gradient(135deg, #4285f4, #9b72cb 55%, #d96570)' },
]

const PARTS: readonly { part: Exclude<RuntimePart, 'compact'>; label: string }[] = [
  { part: 'programs', label: 'rg · git · …' },
  { part: 'browser', label: 'demi browser' },
  { part: 'device', label: 'demi device' },
  { part: 'agent', label: 'demi agent' },
]

const percent = computed(() => Math.round(last.value.context * 100))
const compacting = computed(() => last.value.part === 'compact')
</script>

<template>
  <div class="runtime stage-card" :class="{ shown: card !== null }" aria-hidden="true">
    <div class="model">
      <span class="demi"><span class="otter" v-html="OTTER_SVG" />Demi</span>
      <span class="makers">
        <i v-for="maker in MAKERS" :key="maker.name" class="mark" :title="maker.name" :style="{ maskImage: `url('${maker.url}')`, maskSize: `${maker.fit * 100}%`, background: maker.paint }" />
      </span>
    </div>
    <div class="link"><span>one tool</span></div>
    <div class="shell" :class="{ on: last.part !== null && !compacting }">shell</div>
    <div class="parts">
      <span v-for="each in PARTS" :key="each.part" class="part" :class="{ on: last.part === each.part }">{{ each.label }}</span>
    </div>
    <div class="context" :class="{ compacting }">
      <span class="label">{{ compacting ? 'Compacting' : 'Context' }}</span>
      <span class="bar"><i :style="{ width: `${percent}%` }" /></span>
      <span class="value">{{ percent }}%</span>
    </div>
  </div>
</template>

<style scoped>
/* The card stands where the phone stands on a stage without a scene that uses it. */
.runtime {
  width: 31%;
  min-width: 15rem;
  padding: 1rem 1.1rem 0.95rem;
  border: 1px solid var(--line);
  border-radius: 0.875rem;
  background: var(--paper);
  box-shadow: 0 2px 0.25rem rgb(60 40 20 / 5%), 0 1.25rem 3rem rgb(60 40 20 / 14%);
  font: 500 0.78rem/1.3 var(--sans);
  color: var(--ink);
}

.model {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 0.5rem;
  padding: 0.35rem 0.6rem;
  border: 1px solid var(--line);
  border-radius: 0.5rem;
  background: var(--cream);
}

.demi {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  font-weight: 600;
}

.otter {
  display: block;
  width: 1.35rem;
  height: 1.35rem;
}

.otter :deep(svg) {
  display: block;
  width: 100%;
  height: 100%;
}

.makers {
  display: inline-flex;
  gap: 0.4rem;
}

/* A maker's mark, painted in its brand color through the logo's shape. */
.mark {
  display: block;
  width: 0.95rem;
  height: 0.95rem;
  mask-position: center;
  mask-repeat: no-repeat;
}

/* The one line from the model down to its tool. */
.link {
  position: relative;
  height: 1.35rem;
  margin-left: 50%;
  border-left: 1px dashed var(--line-strong);
}

.link span {
  position: absolute;
  top: 50%;
  left: 0.45rem;
  transform: translateY(-50%);
  color: var(--ink-faint);
  font-size: 0.68rem;
  font-weight: 400;
}

.shell {
  padding: 0.4rem 0.6rem;
  border: 1px solid var(--line-strong);
  border-radius: 0.5rem;
  font-family: var(--mono);
  text-align: center;
  transition: background 0.25s ease, color 0.25s ease, border-color 0.25s ease;
}

.shell.on {
  border-color: var(--button);
  background: var(--button);
  color: var(--paper);
}

.parts {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0.35rem;
  margin-top: 0.55rem;
}

.part {
  padding: 0.3rem 0.45rem;
  border: 1px solid var(--line);
  border-radius: 0.4rem;
  color: var(--ink-soft);
  font: 400 0.7rem/1.3 var(--mono);
  white-space: nowrap;
  transition: background 0.25s ease, color 0.25s ease, border-color 0.25s ease;
}

.part.on {
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 12%, var(--paper));
  color: var(--ink);
}

.context {
  display: grid;
  grid-template-columns: 5.2rem 1fr 2.2rem;
  align-items: center;
  gap: 0.5rem;
  margin-top: 0.8rem;
  color: var(--ink-faint);
  font-size: 0.7rem;
  font-weight: 400;
}

.bar {
  height: 0.3rem;
  border-radius: 999px;
  background: var(--line);
  overflow: hidden;
}

.bar i {
  display: block;
  height: 100%;
  border-radius: inherit;
  background: var(--sage);
  transition: width 0.6s cubic-bezier(0.45, 0, 0.2, 1);
}

.compacting .bar i {
  background: var(--accent);
}

.value {
  text-align: right;
  font-variant-numeric: tabular-nums;
}

@media (prefers-reduced-motion: reduce) {
  .runtime,
  .runtime.shown,
  .bar i {
    transition: none;
  }
}
</style>
