import { computed, type ComputedRef } from 'vue'
import { createThemeStore } from './themeStore'

export const appThemeStore = createThemeStore()

export function useTheme(): { theme: ComputedRef<'light' | 'dark'> } {
  return { theme: computed(() => appThemeStore.state.mode) }
}

export function applyThemeToDocument(): void {
  const apply = (): void => document.documentElement.setAttribute('data-theme', appThemeStore.state.mode)
  apply()
  appThemeStore.subscribe(apply)
}
