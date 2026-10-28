<script setup lang="ts">
// The fourth screen: the models Demi runs on, from the user's own
// subscriptions and keys, with what each subscription has left.
import { ArrowUpIcon, CheckIcon, ChevronDownIcon, KeyRoundIcon, SquareTerminalIcon } from '@lucide/vue'
// The vendors' own marks, the files Demi's settings and model menu show.
import claudeUrl from '@demicodes/web-ui/settings/logos/claude.svg?url&no-inline'
import openaiUrl from '@demicodes/web-ui/settings/logos/openai.svg?url&no-inline'
import xaiUrl from '@demicodes/web-ui/settings/logos/xai.svg?url&no-inline'
import kimiUrl from '@demicodes/web-ui/settings/logos/kimi-code-plan-global.svg?url&no-inline'
import zaiUrl from '@demicodes/web-ui/settings/logos/zai-coding-plan.svg?url&no-inline'
import minimaxUrl from '@demicodes/web-ui/settings/logos/minimax-coding-plan.svg?url&no-inline'

/**
 * A mark, and how much of its box it is drawn across. The files leave
 * different margins around their drawings: Claude's fills its box, OpenAI's
 * and xAI's keep three quarters of theirs. Each is scaled so the drawings
 * look one size, OpenAI's a little larger, as its thin, round knot reads
 * smaller than a star of the same width.
 */
interface Mark {
  url: string
  fit: number
}
const claude: Mark = { url: claudeUrl, fit: 0.92 }
const openai: Mark = { url: openaiUrl, fit: 1.28 }
const xai: Mark = { url: xaiUrl, fit: 1.24 }
const kimi: Mark = { url: kimiUrl, fit: 1 }
const zai: Mark = { url: zaiUrl, fit: 1.24 }
const minimax: Mark = { url: minimaxUrl, fit: 1.24 }

function markStyle(mark: Mark, color?: string): Record<string, string> {
  return { maskImage: `url('${mark.url}')`, maskSize: `${mark.fit * 100}%`, ...(color ? { color } : {}) }
}

const providers = [
  { name: 'Claude', text: 'Use your subscription.', mark: claude, tint: '#f6e2d8', color: '#c9795c' },
  { name: 'ChatGPT', text: 'Use your subscription, Codex included.', mark: openai, tint: '#e2ebdf', color: '#5f7d58' },
  { name: 'Grok Build', text: 'Use your subscription.', mark: xai, tint: '#e1e7ee', color: '#4d6274' },
  {
    name: 'Coding plans',
    text: 'Kimi, GLM, MiniMax and more, with your plan’s key.',
    plans: [
      { mark: kimi, tint: '#e9e5e0', color: '#4a4542' },
      { mark: zai, tint: '#e0e8ef', color: '#56708a' },
      { mark: minimax, tint: '#f4e1e5', color: '#a8586b' },
    ],
  },
  { name: 'API keys', text: 'Anthropic, OpenAI, Gemini or any compatible endpoint.', icon: KeyRoundIcon, tint: '#ebe4f1', color: '#6b5a80' },
  { name: 'Local models', text: 'Run models such as Ollama on your own computer.', icon: SquareTerminalIcon, tint: '#f4e6d2', color: '#8a6235' },
]

const models = [
  { name: 'Claude Opus 5.5', mark: claude, color: '#c9795c', current: false },
  { name: 'GPT-6.1-Sol', mark: openai, color: '#5f7d58', current: true },
  { name: 'Grok 4.7', mark: xai, color: '#4d6274', current: false },
]
const usage = [
  { name: 'Claude', used: 68, detail: 'Resets in 2 h', mark: claude, color: '#c9795c' },
  { name: 'ChatGPT', used: 20, detail: 'Resets in 4 d', mark: openai, color: '#5f7d58' },
  { name: 'Grok Build', used: 45, detail: 'Resets in 6 h', mark: xai, color: '#4d6274' },
]
</script>

<template>
  <section class="providers screen">
    <div class="heading">
      <p class="eyebrow">Your models</p>
      <h2 class="section-title">Bring the AI you already pay for.</h2>
      <p class="section-lede">
        Sign in with your subscriptions, or use coding plans and API keys.<br />
        Add several accounts, and Demi switches to the next when one hits its limit.
      </p>
    </div>

    <div class="grid">
      <article v-for="provider in providers" :key="provider.name" class="card">
        <span v-if="provider.plans" class="plans">
          <span v-for="plan in provider.plans" :key="plan.mark.url" class="plan" :style="{ background: plan.tint, color: plan.color }">
            <i class="mark" :style="markStyle(plan.mark)" />
          </span>
        </span>
        <span v-else class="logo" :style="{ background: provider.tint, color: provider.color }">
          <i v-if="provider.mark" class="mark" :style="markStyle(provider.mark)" />
          <component :is="provider.icon" v-else :size="24" />
        </span>
        <div>
          <h3>{{ provider.name }}</h3>
          <p>{{ provider.text }}</p>
        </div>
      </article>
    </div>

    <div class="extras">
      <div class="extra usage">
        <h3>Keep an eye on usage.</h3>
        <p>See how much of each subscription is used and when it resets.</p>
        <div class="meters">
          <div v-for="item in usage" :key="item.name" class="meter">
            <i class="mark" :style="markStyle(item.mark, item.color)" />
            <span class="name">{{ item.name }}</span>
            <span class="bar"><span :style="{ width: `${item.used}%` }" /></span>
            <span class="value">{{ item.used }}% · {{ item.detail }}</span>
          </div>
        </div>
      </div>
      <div class="extra switch">
        <h3>Switch any time.</h3>
        <p>Change the model or its effort, even in the middle of a conversation.</p>
        <div class="picker" aria-hidden="true">
          <div class="menu">
            <span v-for="model in models" :key="model.name" class="menu-row" :class="{ current: model.current }">
              <i class="mark" :style="markStyle(model.mark, model.color)" />{{ model.name }}
              <CheckIcon v-if="model.current" :size="14" class="check" />
            </span>
          </div>
          <div class="composer">
            <span class="placeholder">Ask anything…</span>
            <span class="model">GPT-6.1-Sol<ChevronDownIcon :size="13" /></span>
            <span class="send"><ArrowUpIcon :size="14" /></span>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.heading {
  margin-bottom: clamp(1.25rem, 3.4vh, 2rem);
  text-align: center;
}

.section-lede {
  margin-right: auto;
  margin-left: auto;
}

.grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 1rem;
  max-width: 76.25rem;
  margin: 0 auto;
  width: 100%;
}

.card {
  display: grid;
  grid-template-columns: 3.25rem 1fr;
  gap: 0.875rem;
  align-items: center;
  padding: 1rem 1.125rem;
  border: 1px solid var(--line);
  border-radius: 1rem;
  background: var(--paper);
}

/* A vendor's mark, monochrome, painted in the text's color as Demi paints it. */
.mark {
  display: inline-block;
  flex: none;
  width: 1em;
  height: 1em;
  background: currentColor;
  mask-position: center;
  mask-repeat: no-repeat;
}

.logo .mark {
  width: 1.5rem;
  height: 1.5rem;
}

/* Several coding plans' marks, in a cluster the size of one logo. */
.plans {
  position: relative;
  width: 3.25rem;
  height: 3.25rem;
}

.plan {
  position: absolute;
  display: grid;
  place-items: center;
  width: 1.75rem;
  height: 1.75rem;
  border: 2px solid var(--paper);
  border-radius: 50%;
}

.plan:nth-child(1) {
  top: -0.125rem;
  left: 0.75rem;
}

.plan:nth-child(2) {
  bottom: -0.125rem;
  left: -0.125rem;
}

.plan:nth-child(3) {
  right: -0.125rem;
  bottom: -0.125rem;
}

.plan .mark {
  width: 0.9375rem;
  height: 0.9375rem;
}

.menu-row .mark {
  width: 0.875rem;
  height: 0.875rem;
}

.logo {
  display: grid;
  place-items: center;
  width: 3.25rem;
  height: 3.25rem;
  border-radius: 50%;
}

h3 {
  font-size: 1.25rem;
  font-weight: 650;
  letter-spacing: -0.03em;
}

.card p {
  margin-top: 0.25rem;
  color: var(--ink-soft);
  font-size: 0.8438rem;
}


.extras {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 1rem;
  max-width: 76.25rem;
  margin: 2.75rem auto 0;
  width: 100%;
}

.extra {
  position: relative;
  display: flex;
  flex-direction: column;
  padding: 1.125rem 1.5rem 1.25rem;
  border: 1px solid var(--line);
  border-radius: 1rem;
  background: var(--paper);
}

.extra h3 {
  font-size: 1.1875rem;
}

.extra p {
  margin-top: 0.25rem;
  color: var(--ink-soft);
  font-size: 0.8438rem;
}

.meters {
  display: flex;
  flex-direction: column;
  gap: 0.625rem;
  margin-top: auto;
  padding-top: 1rem;
  font-size: 0.8125rem;
}

.meter {
  display: grid;
  grid-template-columns: 0.875rem 5.5rem 1fr auto;
  gap: 0.75rem;
  align-items: center;
}

.meter .mark {
  width: 0.875rem;
  height: 0.875rem;
}

.bar {
  position: relative;
  height: 0.5625rem;
  overflow: hidden;
  border-radius: 62.4375rem;
  background: #ece5dc;
}

.bar span {
  position: absolute;
  inset: 0 auto 0 0;
  border-radius: inherit;
  background: var(--ink);
}

.value {
  color: var(--ink-faint);
  font-size: 0.75rem;
}

.picker {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  margin-top: 0.875rem;
  font-size: 0.8125rem;
}

.menu {
  display: flex;
  flex-direction: column;
  width: 12rem;
  margin: 0 3rem 0.375rem 0;
  padding: 0.25rem;
  border: 1px solid var(--line);
  border-radius: 0.75rem;
  background: var(--paper);
  box-shadow: var(--shadow-card);
}

.menu-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.25rem 0.5rem;
  border-radius: 0.4375rem;
}

.menu-row.current {
  background: #f1ebe3;
}

.check {
  margin-left: auto;
}

.composer {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  width: 100%;
  padding: 0.375rem 0.375rem 0.375rem 0.75rem;
  border: 1px solid var(--line-strong);
  border-radius: 0.75rem;
  background: var(--paper);
}

.placeholder {
  flex: 1;
  color: var(--ink-faint);
}

.model {
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
  color: var(--ink-soft);
}

.send {
  display: grid;
  place-items: center;
  width: 1.5rem;
  height: 1.5rem;
  border-radius: 50%;
  background: var(--button);
  color: #fff;
}

@media (max-width: 900px) {
  .grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@media (max-width: 600px) {
  .grid {
    grid-template-columns: minmax(0, 1fr);
  }

  .extras {
    grid-template-columns: minmax(0, 1fr);
  }

  .section-lede br {
    display: none;
  }
}

/* A phone puts each meter's figures under its bar. */
@media (max-width: 560px) {
  .meter {
    grid-template-columns: 0.75rem 5.25rem 1fr;
    row-gap: 0.25rem;
  }

  .value {
    grid-column: 3;
  }
}
</style>
