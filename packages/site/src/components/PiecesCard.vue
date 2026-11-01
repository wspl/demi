<script setup lang="ts">
// Demi's abilities as pieces at the browser's corner, beside a demo whose
// scene says what each request snapped together: a tray of every piece on
// top, and under it each request's recipe, its pieces placed one after
// another, a piece in the tray lighting as it is taken. The same pieces come
// back in recipe after recipe. While the first recipe runs, its pieces light
// one after another, as the work passes along them.
import { computed, ref } from 'vue'
import { BellIcon, ClockIcon, FileTextIcon, GitCommitIcon, GlobeIcon, PenLineIcon, SearchIcon, Volume2Icon } from '@lucide/vue'
import type { Piece, StageCard } from '@/demo/message'
import { useKept } from './stageCard'

type Pieces = Extract<StageCard, { kind: 'pieces' }>

const props = defineProps<{ card: Pieces | null }>()

const last = useKept(() => props.card, { kind: 'pieces', recipes: [], lit: null })

const PIECES: Record<Piece, { label: string; icon: typeof GlobeIcon }> = {
  web: { label: 'Web', icon: GlobeIcon },
  search: { label: 'Search', icon: SearchIcon },
  write: { label: 'Write', icon: PenLineIcon },
  speech: { label: 'Speech', icon: Volume2Icon },
  git: { label: 'Git', icon: GitCommitIcon },
  file: { label: 'File', icon: FileTextIcon },
  schedule: { label: 'Schedule', icon: ClockIcon },
  notify: { label: 'Notify', icon: BellIcon },
}
const TRAY = Object.keys(PIECES) as Piece[]

/** The piece being placed now, which lights in the tray as it is taken. */
const taking = computed<Piece | null>(() => {
  const building = last.value.recipes.at(-1)
  return building && building.placed > 0 && building.placed <= building.pieces.length ? building.pieces[building.placed - 1]! : null
})
/** How many times each piece is in a recipe, which its count in the tray says. */
const uses = computed(() => {
  const counts: Partial<Record<Piece, number>> = {}
  for (const recipe of last.value.recipes)
    for (const piece of recipe.pieces.slice(0, recipe.placed))
      counts[piece] = (counts[piece] ?? 0) + 1
  return counts
})
</script>

<template>
  <div class="pieces stage-card" :class="{ shown: card !== null }" aria-hidden="true">
    <div class="tray">
      <span v-for="piece in TRAY" :key="piece" class="piece" :class="{ taking: taking === piece, used: !!uses[piece] }">
        <component :is="PIECES[piece].icon" :size="12" :stroke-width="2" />
        {{ PIECES[piece].label }}
        <b v-if="(uses[piece] ?? 0) > 1" class="count">×{{ uses[piece] }}</b>
      </span>
    </div>
    <div class="recipes">
      <div v-for="(recipe, r) in last.recipes" :key="recipe.name" class="slot">
      <div class="recipe">
        <span class="name">{{ recipe.name }}</span>
        <span class="chain">
          <template v-for="(piece, i) in recipe.pieces" :key="i">
            <i v-if="i > 0" class="link" :class="{ placed: i < recipe.placed }" />
            <span class="piece placed-piece" :class="{ placed: i < recipe.placed, lit: r === 0 && last.lit === i }">
              <component :is="PIECES[piece].icon" :size="12" :stroke-width="2" />
              {{ PIECES[piece].label }}
            </span>
          </template>
        </span>
      </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/*
 * A new recipe's row opens from nothing to its height, so the card grows
 * upwards smoothly rather than jumping a row at a time.
 */
.pieces {
  display: grid;
  gap: 0.75rem;
  width: 38%;
  min-width: 18rem;
  padding: 0.85rem 0.95rem 0.95rem;
  border: 1px solid var(--line);
  border-radius: 0.875rem;
  background: var(--paper);
  box-shadow: 0 2px 0.25rem rgb(60 40 20 / 5%), 0 1.25rem 3rem rgb(60 40 20 / 14%);
  font: 500 0.7rem/1.2 var(--sans);
  color: var(--ink);
}

.tray {
  display: flex;
  flex-wrap: wrap;
  gap: 0.3rem;
  padding-bottom: 0.7rem;
  border-bottom: 1px dashed var(--line-strong);
}

.piece {
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: 0.3rem;
  padding: 0.28rem 0.5rem;
  border: 1px solid var(--line);
  border-radius: 0.45rem;
  background: var(--cream);
  color: var(--ink-soft);
  white-space: nowrap;
  transition: border-color 0.25s ease, background 0.25s ease, color 0.25s ease, transform 0.25s ease;
}

.piece.used {
  color: var(--ink);
}

/* The piece a request takes now lifts in the tray. */
.piece.taking {
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 12%, var(--paper));
  transform: translateY(-2px);
}

.count {
  margin-left: 0.1rem;
  color: var(--accent);
  font-weight: 600;
}

.recipes {
  display: grid;
  margin-top: -0.55rem;
}

/* A row's room opens as its grid row grows from 0fr; its content fades in once there is room. */
.slot {
  display: grid;
  grid-template-rows: 1fr;
  animation: row-open 0.45s cubic-bezier(0.2, 0, 0, 1) both;
}

@keyframes row-open {
  from {
    grid-template-rows: 0fr;
  }
}

.recipe {
  display: grid;
  gap: 0.3rem;
  min-height: 0;
  overflow: hidden;
  padding-top: 0.55rem;
  animation: row-in 0.3s ease-out 0.15s both;
}

@keyframes row-in {
  from {
    opacity: 0;
  }
}

.name {
  color: var(--ink-faint);
  font-weight: 500;
}

.chain {
  display: flex;
  align-items: center;
}

/* A piece in a recipe snaps into place: it drops in as it is placed. */
.placed-piece {
  opacity: 0;
  transform: translateY(-0.5rem) scale(0.9);
  transition:
    opacity 0.25s ease,
    transform 0.35s cubic-bezier(0.34, 1.56, 0.64, 1),
    border-color 0.25s ease,
    background 0.25s ease,
    color 0.25s ease;
}

.placed-piece.placed {
  opacity: 1;
  transform: none;
  color: var(--ink);
}

/* The piece the work is at, while a recipe runs. */
.placed-piece.lit {
  border-color: var(--accent);
  background: var(--accent);
  color: var(--paper);
}

.link {
  width: 0.7rem;
  height: 1px;
  background: var(--line-strong);
  opacity: 0;
  transition: opacity 0.25s ease;
}

.link.placed {
  opacity: 1;
}

@media (prefers-reduced-motion: reduce) {
  .pieces,
  .pieces.shown,
  .placed-piece {
    transition: none;
  }

  .slot,
  .recipe {
    animation: none;
  }
}
</style>
