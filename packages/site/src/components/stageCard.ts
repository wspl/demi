// A card a scene shows over the stage's browser, at its lower right corner
// (RuntimeCard, ProductionCard, PiecesCard): its root takes the `stage-card`
// class (`style.css`), which comes in from the right as it fades in and
// leaves the same way, and it draws what `useKept` keeps, so it leaves
// showing what it showed rather than emptying as it goes.
import { ref, watch, type Ref } from 'vue'

/** The last card `card` gave, from `initial` until it gives one. */
export function useKept<T>(card: () => T | null, initial: T): Readonly<Ref<T>> {
  const kept = ref(initial) as Ref<T>
  watch(card, (next) => {
    if (next !== null)
      kept.value = next
  })
  return kept
}
