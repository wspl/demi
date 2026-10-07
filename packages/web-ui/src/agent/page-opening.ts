import { inject, provide, type InjectionKey } from 'vue'
import type { PresentedPage } from '../plugins/intents'

/** Opens a page the agent presented, through the `page` intent (`plugin-pages.md` § Intents). */
export type PageOpeningHandler = (page: PresentedPage) => void
const pageOpeningKey: InjectionKey<() => PageOpeningHandler | undefined> = Symbol('page-opening')

/**
 * Where the transcript below sends a presented page's Open, read at each
 * render: none while no plugin the user has on opens a page, and the cards
 * then offer no Open.
 */
export function providePageOpening(handler: () => PageOpeningHandler | undefined): void {
  provide(pageOpeningKey, handler)
}

export function usePageOpening(): () => PageOpeningHandler | undefined {
  return inject(pageOpeningKey, () => undefined)
}
