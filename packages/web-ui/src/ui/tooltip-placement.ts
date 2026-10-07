import { inject, provide, type InjectionKey } from 'vue'
import type { Placement } from '@floating-ui/vue'

/** Where the tips inside a component open when they name no side of their own. */
const tooltipPlacementKey: InjectionKey<Placement> = Symbol('tooltip-placement')

/**
 * The tips inside the calling component open on `placement` unless they name
 * their own side: a bar right under a tab strip opens its tips below, so no
 * tip covers the strip's tabs.
 */
export function provideTooltipPlacement(placement: Placement): void {
  provide(tooltipPlacementKey, placement)
}

/** The side a tip opens on when it names none: its surroundings', else above. */
export function injectTooltipPlacement(): Placement {
  return inject(tooltipPlacementKey, 'top')
}
