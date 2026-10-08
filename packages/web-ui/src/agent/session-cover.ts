import type { InjectionKey, Ref } from 'vue'

/**
 * The height of what stands over the dock's top without taking room in it,
 * such as a waiting permission card (`permissions.md` § What the user sees):
 * the dock (`SessionDock`) sets it, and the surface (`SessionSurface`) keeps
 * the transcript clear of it while a panel the dock opened keeps its place
 * under it.
 */
export const sessionCoverKey: InjectionKey<Ref<number>> = Symbol('session-cover')
