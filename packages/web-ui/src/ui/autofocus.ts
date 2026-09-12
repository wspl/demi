import { hasInjectionContext, inject, type InjectionKey, type Ref } from 'vue'

export const autofocusEnabledKey: InjectionKey<Readonly<Ref<boolean>>> =
  Symbol('autofocus-enabled')

/** Suppresses automatic focus in a multi-form surface, without blocking user focus. */
export function useAutofocus(): (
  element: HTMLElement | null | undefined,
) => void {
  const enabled = hasInjectionContext() ? inject(autofocusEnabledKey, null) : null
  return (element) => {
    if (enabled?.value === false) {
      return
    }
    element?.focus()
  }
}
