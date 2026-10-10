import type { InjectionKey, Ref } from 'vue'

/**
 * Where the dock (`SessionDock`) stands its cards: above the chips, which sit
 * directly on the input (`product.md` § Recovering an unfinished turn). The
 * composer, which holds the input, puts its cards there, such as the offline
 * primary Host's card, so nothing comes between the chips and the input; a
 * composer outside a dock finds none and stands its cards over its own input.
 */
export const dockCardsKey: InjectionKey<Readonly<Ref<HTMLElement | undefined>>> = Symbol('dock-cards')
