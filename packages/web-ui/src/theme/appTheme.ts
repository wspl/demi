import { computed, type ComputedRef } from 'vue'
import { createThemeStore, type ThemeStoreState } from './themeStore'

type ThemeMode = ThemeStoreState['mode']

const STORAGE_KEY = 'demi-theme-mode'

function systemPrefersLight(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-color-scheme: light)').matches
  )
}

function storedMode(): ThemeMode | null {
  if (typeof localStorage === 'undefined') return null
  const value = localStorage.getItem(STORAGE_KEY)
  return value === 'light' || value === 'dark' ? value : null
}

/** Initial mode: an explicit saved choice, else the OS preference, else dark. */
function initialMode(): ThemeMode {
  return storedMode() ?? (systemPrefersLight() ? 'light' : 'dark')
}

export const appThemeStore = createThemeStore({ mode: initialMode() })

export function useTheme(): { theme: ComputedRef<ThemeMode> } {
  return { theme: computed(() => appThemeStore.state.mode) }
}

/** Set the theme and remember the choice (so it survives reloads and stops following the OS). */
export function setTheme(mode: ThemeMode): void {
  appThemeStore.setMode(mode)
  if (typeof localStorage !== 'undefined') localStorage.setItem(STORAGE_KEY, mode)
}

/** Flip between light and dark, remembering the choice. */
export function toggleTheme(): void {
  setTheme(appThemeStore.state.mode === 'dark' ? 'light' : 'dark')
}

/** Mirror the active mode onto `<html data-theme>`, and follow the OS until the user chooses explicitly. */
export function applyThemeToDocument(): void {
  const apply = (): void => document.documentElement.setAttribute('data-theme', appThemeStore.state.mode)
  apply()
  appThemeStore.subscribe(apply)

  if (typeof window !== 'undefined' && typeof window.matchMedia === 'function') {
    window.matchMedia('(prefers-color-scheme: light)').addEventListener('change', (event) => {
      if (storedMode()) return // user made an explicit choice — don't override it
      appThemeStore.setMode(event.matches ? 'light' : 'dark')
    })
  }
}
