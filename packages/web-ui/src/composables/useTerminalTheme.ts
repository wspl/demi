import { computed, type ComputedRef } from 'vue'
import { useTheme } from '../theme/appTheme'

export interface TerminalTheme {
  foreground: string
  background: string
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

const darkTheme: TerminalTheme = {
  background: '#171717',
  foreground: '#d4d4d8',
  black: '#171717',
  red: '#f87171',
  green: '#4ade80',
  yellow: '#facc15',
  blue: '#60a5fa',
  magenta: '#c084fc',
  cyan: '#22d3ee',
  white: '#d4d4d8',
  brightBlack: '#525252',
  brightRed: '#fca5a5',
  brightGreen: '#86efac',
  brightYellow: '#fde68a',
  brightBlue: '#93c5fd',
  brightMagenta: '#d8b4fe',
  brightCyan: '#67e8f9',
  brightWhite: '#fafafa',
}

const lightTheme: TerminalTheme = {
  background: '#f9fafb',
  foreground: '#1e1e1e',
  black: '#1e1e1e',
  red: '#dc2626',
  green: '#16a34a',
  yellow: '#a16207',
  blue: '#2563eb',
  magenta: '#9333ea',
  cyan: '#0891b2',
  white: '#d4d4d8',
  brightBlack: '#737373',
  brightRed: '#ef4444',
  brightGreen: '#22c55e',
  brightYellow: '#ca8a04',
  brightBlue: '#3b82f6',
  brightMagenta: '#a855f7',
  brightCyan: '#06b6d4',
  brightWhite: '#f5f5f5',
}

let terminalTheme: ComputedRef<TerminalTheme> | undefined

export function useTerminalTheme(): { terminalTheme: ComputedRef<TerminalTheme> } {
  if (!terminalTheme) {
    const { theme } = useTheme()
    terminalTheme = computed<TerminalTheme>(() => (theme.value === 'dark' ? darkTheme : lightTheme))
  }
  return { terminalTheme }
}
