import type { InjectionKey, Ref } from 'vue'

/**
 * The height of what stands over the dock's top without taking room in it,
 * such as a waiting permission card: the dock (`SessionDock`) sets it, and
 * the surface (`SessionSurface`) keeps the transcript clear of it while a
 * panel the dock opened keeps its place under it.
 */
export const sessionCoverKey: InjectionKey<Ref<number>> = Symbol('session-cover')

/**
 * The height of the dock's composer, from its top to the dock's foot: the dock
 * (`SessionDock`) sets it, and the surface's veil (`SessionBottomVeil`) fades
 * over the chips row above it.
 */
export const sessionComposerKey: InjectionKey<Ref<number>> = Symbol('session-composer')
