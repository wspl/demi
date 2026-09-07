import type { InjectionKey } from 'vue'

/** Provided by a dialog to its subtree, so a dialog opened there stacks instead of replacing it. */
export const dialogNestingKey: InjectionKey<boolean> = Symbol('dialogNesting')
