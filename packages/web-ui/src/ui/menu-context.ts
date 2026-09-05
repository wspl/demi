import type { ComputedRef, InjectionKey } from 'vue'

export const menuIconlessKey: InjectionKey<ComputedRef<boolean>> = Symbol('menuIconless')

export interface MenuRoot {
  dismiss: () => void
}

export const menuRootKey: InjectionKey<MenuRoot> = Symbol('menuRoot')

/** Action rows dismiss the tree. Choices, flyout hosts, and suffix controls stay. */
export function shouldDismissMenuTree(item: {
  isChoice: boolean
  hasSubmenu: boolean
  hasSuffix: boolean
}): boolean {
  return !item.isChoice && !item.hasSubmenu && !item.hasSuffix
}
