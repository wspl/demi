import { computed, type ComputedRef } from 'vue'
import { useTheme } from '../theme/appTheme'
import type { PromptColors, PromptRun } from '../agent/terminals'
import type { CodeRun } from '../markdown/highlight'

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

/** How a span is drawn: its color as `rgb(r, g, b)`, and whether it is italic and bold. */
interface ProbedStyle {
  color: string
  italic: boolean
  bold: boolean
}

/**
 * How spans are drawn under `root`, each set up by `dress` with one of
 * `values`; the color as `rgb(r, g, b)`, whatever notation it is written
 * in. Computed style keeps a color's own notation (`oklch(…)`,
 * `color(srgb …)`), which xterm cannot paint, so each color is drawn on a
 * one-pixel canvas and read back as sRGB. All of them are probed at once, so
 * the page computes its style once for them, not once for each.
 */
function probeStyles<T>(root: Element, values: readonly T[], dress: (probe: HTMLElement, value: T) => void): ProbedStyle[] {
  const probes = values.map((value) => {
    const probe = document.createElement('span')
    dress(probe, value)
    root.appendChild(probe)
    return probe
  })
  // Read while the probes are in the page: a removed element has no computed style.
  const computed = probes.map((probe) => {
    const style = getComputedStyle(probe)
    return { color: style.color, italic: style.fontStyle === 'italic', bold: Number(style.fontWeight) >= 600 }
  })
  probes.forEach((probe) => probe.remove())
  const context = document.createElement('canvas').getContext('2d', { willReadFrequently: true })
  // No 2D context (a headless renderer without one): keep what the style gave.
  if (!context)
    return computed
  return computed.map((style) => {
    context.clearRect(0, 0, 1, 1)
    context.fillStyle = style.color
    context.fillRect(0, 0, 1, 1)
    const [r, g, b] = context.getImageData(0, 0, 1, 1).data
    return { ...style, color: `rgb(${r}, ${g}, ${b})` }
  })
}

/** CSS colors as `rgb(r, g, b)`, whatever notation each is written in (`probeStyles`). */
function resolveCssColors(root: Element, values: readonly string[]): string[] {
  return probeStyles(root, values, (probe, value) => {
    probe.style.color = value
  }).map((style) => style.color)
}

/** A CSS `rgb()` color as the escape sequence that sets it as the foreground, or nothing for a color it cannot read. */
export function cssColorToAnsiForeground(color: string): string {
  const match = color.match(RGB)
  if (!match)
    return ''
  return `\x1b[38;2;${Math.round(Number(match[1]))};${Math.round(Number(match[2]))};${Math.round(Number(match[3]))}m`
}

export function terminalPalette(mode: 'dark' | 'light'): TerminalTheme {
  return mode === 'light' ? lightTheme : darkTheme
}

/** The color tokens a terminal takes from the page. */
const TERMINAL_TOKENS = ['--surface', '--fg-body', '--fg-emphasis', '--on-accent', '--fg-muted'] as const

/**
 * A terminal's colors from the live tokens under `el`, in one look at the
 * page's style. The theme: ANSI 16 stay on the palette so red stays red when
 * the accent is purple; ground, cursor and selection follow the tokens
 * (tone, accent, surface). The prompt line's colors (`promptLine`): the
 * prompt in the muted text color, the script in `codeText`, code's text
 * color. A token that does not resolve keeps the palette's color.
 */
export function terminalColors(
  el: HTMLElement,
  palette: TerminalTheme,
  codeText: string,
): { theme: TerminalTheme; prompt: PromptColors } {
  const resolved = resolveCssColors(el, [...TERMINAL_TOKENS.map((name) => `var(${name})`), codeText])
  const token = (name: typeof TERMINAL_TOKENS[number], fallback: string): string => {
    const color = resolved[TERMINAL_TOKENS.indexOf(name)]
    return color !== undefined && RGB.test(color) ? color : fallback
  }
  const script = resolved[TERMINAL_TOKENS.length]
  const background = token('--surface', palette.background)
  const accent = token('--on-accent', palette.blue)
  const selection = RGB.test(accent)
    ? cssColorToRgba(accent, 0.34)
    : palette.selectionBackground
  return {
    theme: {
      ...palette,
      background,
      foreground: token('--fg-body', palette.foreground),
      cursor: token('--fg-emphasis', palette.cursor),
      cursorAccent: background,
      selectionBackground: selection,
      selectionInactiveBackground: selection,
      overviewRulerBorder: background,
    },
    prompt: {
      prompt: cssColorToAnsiForeground(token('--fg-muted', palette.brightBlack)),
      script: cssColorToAnsiForeground(script !== undefined && RGB.test(script) ? script : palette.brightWhite),
    },
  }
}

/**
 * Highlighted runs of code as the escape sequences that draw them in a
 * terminal: the color its classes give each run, read as sRGB under `el`,
 * and its italic and bold. The distinct classes are read in one look at the
 * page's style.
 */
export function terminalCodeRuns(el: HTMLElement, runs: readonly CodeRun[]): PromptRun[] {
  const distinct = [...new Set(runs.map((run) => run.classes))]
  const styles = probeStyles(el, distinct, (probe, classes) => {
    probe.className = classes
  })
  const escapes = new Map(distinct.map((classes, index) => {
    const style = styles[index]!
    return [classes, `${style.italic ? '\x1b[3m' : ''}${style.bold ? '\x1b[1m' : ''}${cssColorToAnsiForeground(style.color)}`]
  }))
  return runs.flatMap((run) => {
    const ansi = escapes.get(run.classes) ?? ''
    return ansi ? [{ from: run.from, to: run.to, ansi }] : []
  })
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
