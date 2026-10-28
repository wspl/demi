<script setup lang="ts">
// The second screen: a tour of what using Demi is like. The list on the left
// works as tabs: the selected feature opens to its description, and its demo
// plays on the right. The tour moves to the next feature when the progress bar
// fills; a reader who asks for reduced motion moves it by hand. Once the page
// runs, every feature holds all the titles and descriptions, stacked and all
// but their own hidden, so each is as tall as its longest: the closing feature
// shrinks by what the opening one grows, and nothing else on the screen moves
// when the selection does. The page's HTML holds each text once, as search
// engines read it: the hidden copies come only after it mounts.
import { ChevronDownIcon } from '@lucide/vue'
import { computed, onMounted, ref } from 'vue'
import DemoStage from '@/components/DemoStage.vue'
import { FEATURES } from '@/features'

const selected = ref(0)
const mounted = ref(false)
onMounted(() => {
  mounted.value = true
})
const feature = computed(() => FEATURES[selected.value]!)

function next(): void {
  selected.value = (selected.value + 1) % FEATURES.length
}

const tabs = ref<HTMLButtonElement[]>([])

/** The arrow keys, Home and End move the selection and the focus, as a tab list's do. */
function move(event: KeyboardEvent): void {
  const last = FEATURES.length - 1
  const to = {
    ArrowDown: selected.value === last ? 0 : selected.value + 1,
    ArrowUp: selected.value === 0 ? last : selected.value - 1,
    Home: 0,
    End: last,
  }[event.key]
  if (to === undefined)
    return
  event.preventDefault()
  selected.value = to
  tabs.value[to]?.focus()
}
</script>

<template>
  <section id="features" class="tour screen">
    <div class="heading">
      <p class="eyebrow">Features</p>
      <h2 class="section-title">Built for real work.</h2>
    </div>

    <div class="body">
      <div class="list" role="tablist" aria-orientation="vertical" aria-label="Features" @keydown="move">
        <button
          v-for="(item, index) in FEATURES"
          :id="`feature-tab-${item.id}`"
          :key="item.id"
          ref="tabs"
         
          class="item"
          :class="{ selected: index === selected }"
          role="tab"
          type="button"
          :aria-selected="index === selected"
          :tabindex="index === selected ? 0 : -1"
          aria-controls="feature-panel"
          @click="selected = index"
        >
          <img class="icon" :src="item.icon" alt="" width="48" height="48" />
          <span class="title stack">
            <template v-for="other in FEATURES" :key="other.id">
              <span v-if="other.id === item.id" class="shown">{{ other.title }}</span>
              <span v-else-if="mounted" aria-hidden="true">{{ other.title }}</span>
            </template>
          </span>
          <ChevronDownIcon class="chevron" :size="18" />
          <span class="details" :aria-hidden="index !== selected">
            <span class="reveal">
            <span class="description stack">
              <template v-for="other in FEATURES" :key="other.id">
                <span v-if="other.id === item.id" class="shown">{{ other.description }}</span>
                <span v-else-if="mounted" aria-hidden="true">{{ other.description }}</span>
              </template>
            </span>
            <span class="progress" aria-hidden="true">
              <span
                v-if="index === selected"
                :key="selected"
                class="fill"
                :style="{ animationDuration: `${item.seconds}s` }"
                @animationend="next"
              />
            </span>
            </span>
          </span>
        </button>
      </div>

      <div id="feature-panel" class="panel" role="tabpanel" :aria-labelledby="`feature-tab-${feature.id}`">
        <DemoStage :demo="feature.id" :label="feature.title" />
      </div>
    </div>
  </section>
</template>

<style scoped>
.heading {
  margin-bottom: clamp(1.25rem, 3.4vh, 2rem);
  text-align: center;
}

.body {
  display: grid;
  grid-template-columns: minmax(18.75rem, 32%) 1fr;
  gap: clamp(1.5rem, 2.6vw, 2.5rem);
  align-items: center;
  width: 100%;
  max-width: 93.75rem;
  margin: 0 auto;
}

.list {
  display: flex;
  flex-direction: column;
}

/*
 * A feature opens and closes in the same time and curve as the one it
 * replaces, so the two always add up to the same height.
 */
.item {
  --open: 0.45s cubic-bezier(0.2, 0, 0, 1);
  position: relative;
  display: grid;
  grid-template-columns: 3rem 1fr 1.25rem;
  column-gap: 1rem;
  align-content: start;
  align-items: center;
  padding: 0.75rem 1rem;
  border: 0;
  border-radius: 1rem;
  background: transparent;
  text-align: left;
  cursor: pointer;
  transition:
    padding var(--open),
    background-color 0.3s ease;
}

/* The line between two features, left out around the open one. */
.item::before {
  position: absolute;
  top: 0;
  right: 0;
  left: 0;
  height: 1px;
  background: var(--line);
  content: "";
  transition: opacity 0.3s ease;
}

.item:first-child::before,
.item.selected::before,
.item.selected + .item::before {
  opacity: 0;
}

.item.selected {
  padding: 1.125rem 1rem;
  background: #f3eee6;
}

.icon {
  width: 3rem;
  height: 3rem;
  object-fit: contain;
}

/* Texts in one cell, only the shown one visible. */
.stack {
  display: grid;
}

.stack > * {
  grid-area: 1 / 1;
  visibility: hidden;
}

.stack > .shown {
  visibility: visible;
}

.title {
  min-width: 0;
  color: #4a3f39;
  font-family: var(--serif);
  font-size: clamp(1.125rem, 1.4vw, 1.3125rem);
  font-weight: 600;
  letter-spacing: -0.02em;
  line-height: 1.15;
  transition: color 0.3s ease;
}

.selected .title {
  color: var(--ink);
}

.chevron {
  color: var(--ink);
  transition: transform var(--open);
}

.selected .chevron {
  transform: rotate(180deg);
}

/* The description opens from no height to its own. */
.details {
  display: grid;
  grid-column: 2;
  grid-template-rows: 0fr;
  opacity: 0;
  transition:
    grid-template-rows var(--open),
    opacity 0.2s ease;
}

.selected .details {
  grid-template-rows: 1fr;
  opacity: 1;
  transition:
    grid-template-rows var(--open),
    opacity 0.3s ease 0.12s;
}

.reveal {
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
}

.description {
  padding-top: 0.5rem;
  color: var(--ink-soft);
  font-size: 0.9062rem;
  line-height: 1.55;
}

.progress {
  position: relative;
  flex: none;
  height: 0.375rem;
  margin-top: 0.875rem;
  overflow: hidden;
  border-radius: 62.4375rem;
  background: #e5ddd2;
}

.fill {
  position: absolute;
  inset: 0;
  border-radius: inherit;
  background: var(--ink);
  transform-origin: left;
  animation: fill 12s linear forwards;
}

@keyframes fill {
  from {
    transform: scaleX(0);
  }

  to {
    transform: scaleX(1);
  }
}

@media (prefers-reduced-motion: reduce) {
  .progress {
    display: none;
  }
}

@media (max-width: 960px) {
  .body {
    grid-template-columns: minmax(0, 1fr);
  }

  .panel {
    order: -1;
  }
}
</style>
