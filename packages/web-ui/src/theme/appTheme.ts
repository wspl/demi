import { computed, ref, type ComputedRef } from 'vue'
import { createThemeStore, type ThemeStoreState } from './themeStore'

export type ThemeMode = ThemeStoreState['mode']
/** What the user asked for: a mode, or to follow the OS. */
export type ThemeChoice = ThemeMode | 'system'

const STORAGE_KEY = 'demi-theme-mode'

function systemPrefersLight(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-color-scheme: light)').matches
  )
}

function storedMode(): ThemeMode | null {
  try {
    const value = localStorage.getItem(STORAGE_KEY)
    return value === 'light' || value === 'dark' ? value : null
  } catch {
    // Theme restoration is optional when storage access is blocked.
    return null
  }
}

const choice = ref<ThemeChoice>(storedMode() ?? 'system')

/** Initial mode: an explicit saved choice, else the OS preference, else dark. */
function initialMode(): ThemeMode {
  return choice.value === 'system'
    ? (systemPrefersLight() ? 'light' : 'dark')
    : choice.value
}

export const appThemeStore = createThemeStore({ mode: initialMode() })

export function useTheme(): { theme: ComputedRef<ThemeMode> } {
  return { theme: computed(() => appThemeStore.state.mode) }
}

/** Set the theme and remember the choice (so it survives reloads and stops following the OS). */
export function setTheme(mode: ThemeMode): void {
  choice.value = mode
  appThemeStore.setMode(mode)
  try {
    localStorage.setItem(STORAGE_KEY, mode)
  } catch {
    // Keep the selected theme in memory when storage is unavailable.
  }
}

/** Flip between light and dark, remembering the choice. */
export function toggleTheme(): void {
  setTheme(appThemeStore.state.mode === 'dark' ? 'light' : 'dark')
}

/** Mirror the active mode onto `<html data-theme>`, and follow the OS until the user chooses explicitly. */
export function applyThemeToDocument(): () => void {
  const apply = (): void =>
    document.documentElement.setAttribute('data-theme', appThemeStore.state.mode)
  apply()
  const unsubscribe = appThemeStore.subscribe(apply)
  const media = typeof window.matchMedia === 'function'
    ? window.matchMedia('(prefers-color-scheme: light)')
    : null
  const change = (event: MediaQueryListEvent) => {
    if (choice.value === 'system')
      appThemeStore.setMode(event.matches ? 'light' : 'dark')
  }
  media?.addEventListener('change', change)
  return () => {
    unsubscribe()
    media?.removeEventListener('change', change)
  }
}

/** The saved choice, or `system` while the theme follows the OS. */
export function themeChoice(): ThemeChoice {
  return choice.value
}

/** A mode is remembered; `system` forgets the choice and follows the OS from now on. */
export function setThemeChoice(next: ThemeChoice): void {
  if (next !== 'system') {
    setTheme(next)
    return
  }
  choice.value = 'system'
  try {
    localStorage.removeItem(STORAGE_KEY)
  } catch {
    // The current page can still follow the system theme.
  }
  appThemeStore.setMode(systemPrefersLight() ? 'light' : 'dark')
}
