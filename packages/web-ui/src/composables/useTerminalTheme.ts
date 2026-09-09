import { computed, type ComputedRef } from 'vue'
import { useTheme } from '../theme/appTheme'

/** 16 ANSI colors plus the xterm chrome (cursor and selection). */
export interface TerminalTheme {
  foreground: string
  background: string
  cursor: string
  cursorAccent: string
  selectionBackground: string
  selectionInactiveBackground: string
  overviewRulerBorder: string
  black: string
  red: string
  green: string
  yellow: string
  blue: string
  magenta: string
  cyan: string
  white: string
  brightBlack: string
  brightRed: string
  brightGreen: string
  brightYellow: string
  brightBlue: string
  brightMagenta: string
  brightCyan: string
  brightWhite: string
}

/** Dark: body text on `--surface`. Brights are lighter, still clear of the ground. */
const darkTheme: TerminalTheme = {
  background: '#171717',
  foreground: '#d4d4d4',
  cursor: '#f5f5f5',
  cursorAccent: '#171717',
  selectionBackground: 'rgba(96, 165, 250, 0.34)',
  selectionInactiveBackground: 'rgba(96, 165, 250, 0.34)',
  overviewRulerBorder: '#171717',
  black: '#0a0a0a',
  red: '#f87171',
  green: '#34d399',
  yellow: '#fbbf24',
  blue: '#60a5fa',
  magenta: '#a78bfa',
  cyan: '#22d3ee',
  white: '#d4d4d4',
  brightBlack: '#737373',
  brightRed: '#fca5a5',
  brightGreen: '#6ee7b7',
  brightYellow: '#fde68a',
  brightBlue: '#93c5fd',
  brightMagenta: '#c4b5fd',
  brightCyan: '#67e8f9',
  brightWhite: '#f5f5f5',
}

/** Light: brights are darker and more saturated, not washed toward the paper. */
const lightTheme: TerminalTheme = {
  background: '#f9fafb',
  foreground: '#404550',
  cursor: '#1a1d24',
  cursorAccent: '#f9fafb',
  selectionBackground: 'rgba(37, 99, 235, 0.34)',
  selectionInactiveBackground: 'rgba(37, 99, 235, 0.34)',
  overviewRulerBorder: '#f9fafb',
  black: '#1a1d24',
  red: '#dc2626',
  green: '#16a34a',
  yellow: '#d97706',
  blue: '#2563eb',
  magenta: '#7c3aed',
  cyan: '#0e7490',
  white: '#404550',
  brightBlack: '#626873',
  brightRed: '#b91c1c',
  brightGreen: '#15803d',
  brightYellow: '#b45309',
  brightBlue: '#1d4ed8',
  brightMagenta: '#6d28d9',
  brightCyan: '#155e75',
  brightWhite: '#1a1d24',
}

const RGB = /rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)/

export function cssColorToRgba(color: string, alpha: number): string {
  const match = color.match(RGB)
  if (!match)
    return color
  return `rgba(${Number(match[1])}, ${Number(match[2])}, ${Number(match[3])}, ${alpha})`
}

export function resolveCssColor(root: Element, value: string): string {
  const probe = document.createElement('span')
  probe.style.color = value
  root.appendChild(probe)
  const resolved = getComputedStyle(probe).color
  probe.remove()
  return resolved
}

export function terminalPalette(mode: 'dark' | 'light'): TerminalTheme {
  return mode === 'light' ? lightTheme : darkTheme
}

/**
 * ANSI 16 stay on the palette so red stays red when the accent is purple.
 * Ground, cursor and selection follow the live tokens (tone, accent, surface).
 */
export function xtermThemeFromElement(el: HTMLElement, palette: TerminalTheme): TerminalTheme {
  const token = (name: string, fallback: string) => {
    const resolved = resolveCssColor(el, `var(${name})`)
    return RGB.test(resolved) ? resolved : fallback
  }
  const background = token('--surface', palette.background)
  const foreground = token('--fg-body', palette.foreground)
  const emphasis = token('--fg-emphasis', palette.cursor)
  const accent = token('--on-accent', palette.blue)
  const selection = RGB.test(accent)
    ? cssColorToRgba(accent, 0.34)
    : palette.selectionBackground
  return {
    ...palette,
    background,
    foreground,
    cursor: emphasis,
    cursorAccent: background,
    selectionBackground: selection,
    selectionInactiveBackground: selection,
    overviewRulerBorder: background,
  }
}

let terminalTheme: ComputedRef<TerminalTheme> | undefined

export function useTerminalTheme(): {
  terminalTheme: ComputedRef<TerminalTheme>
} {
  if (!terminalTheme) {
    const { theme } = useTheme()
    terminalTheme = computed(() => terminalPalette(theme.value))
  }
  return { terminalTheme }
}
