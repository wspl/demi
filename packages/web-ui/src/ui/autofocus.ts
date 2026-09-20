import { hasInjectionContext, inject, type InjectionKey, type Ref } from 'vue'

export const autofocusEnabledKey: InjectionKey<Readonly<Ref<boolean>>> =
  Symbol('autofocus-enabled')

/**
 * Suppresses automatic focus in a multi-form surface, without blocking user
 * focus. The target is an element, or anything that takes the focus the way
 * its own kind should, such as an editor that puts its cursor at the end.
 */
export function useAutofocus(): (
  target: Pick<HTMLElement, 'focus'> | null | undefined,
) => void {
  const enabled = hasInjectionContext() ? inject(autofocusEnabledKey, null) : null
  return (target) => {
    if (enabled?.value === false) {
      return
    }
    target?.focus()
  }
}
