import type { InjectionKey } from 'vue'

/** Provided inside a bottom sheet (`Sheet.vue`): a menu there lays its rows out at a finger's height. */
export const inSheetKey: InjectionKey<boolean> = Symbol('inSheet')
