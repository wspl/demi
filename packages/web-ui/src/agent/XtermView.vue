<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { terminalCodeRuns, terminalColors, useTerminalTheme } from '../composables/useTerminalTheme'
import { codeRuns, codeTextColor, useMarkdownRenderVersion } from '../markdown/highlight'
import { promptLine, terminalWrite, type PromptColors, type TerminalShown } from './terminals'
import '@xterm/xterm/css/xterm.css'

const props = defineProps<{
  output: string
  /**
   * The live view's count of characters at the end of `output`; absent for
   * output that is whole, such as a transcript's.
   */
  chars?: number
  running?: boolean
  /** The command's script, which the terminal opens with after a prompt; the page's line, not the output's. */
  script?: string
}>()

/**
 * The element the terminal opens in and fits to. It has no padding of its
 * own, which the fit would count as room (the fit addon measures the
 * element's border box): the padding is its parent's.
 */
const hostRef = ref<HTMLElement | null>(null)
const { terminalTheme } = useTerminalTheme()

let term: Terminal | undefined
let fit: FitAddon | undefined
/** The output the terminal shows, and the live view's count at its end; null before anything. */
let shown: TerminalShown | null = null
/** The theme the terminal draws in, as JSON, so colors that did not change are not drawn again. */
let drawnTheme = ''
/** The prompt line's colors, from the same look at the tokens as the theme. */
let promptColors: PromptColors | null = null
/** The prompt line, in the colors of the theme it was last drawn in. */
let prompt = ''
let resize: ResizeObserver | undefined
let appearance: MutationObserver | undefined

/**
 * Takes the colors from the live tokens and draws what they change: the
 * theme, and the prompt line with what follows it. A theme the terminal
 * already draws is not set again, since xterm then repaints every row and
 * restyles the page.
 */
function applyColors(): void {
  const host = hostRef.value
  if (!term || !host) {
    return
  }
  const colors = terminalColors(host, terminalTheme.value, codeTextColor())
  const theme = JSON.stringify(colors.theme)
  if (theme !== drawnTheme) {
    drawnTheme = theme
    term.options.theme = colors.theme
    host.style.setProperty('--xterm-selection', colors.theme.selectionBackground)
  }
  promptColors = colors.prompt
  applyPrompt()
}

/**
 * Clears the terminal in the order of its writes. xterm.js's `reset()` acts
 * at once, while a write is queued and parsed later, so a write still queued
 * would land after the reset: a prompt written just before the script's
 * highlighting arrived showed twice. ESC c, a full reset, is parsed in turn.
 */
const FULL_RESET = '\x1bc'

/**
 * Takes the prompt line for the script in the live colors. Its colors are
 * written into the terminal, so a new theme or script draws it, and the
 * output after it, anew.
 */
function applyPrompt(): void {
  if (!term || !promptColors) {
    return
  }
  const host = hostRef.value
  if (!host) {
    return
  }
  // The script highlighted as Bash, in the colors all code shows in.
  const runs = props.script ? terminalCodeRuns(host, codeRuns(props.script.replace(/(\r?\n)+$/, ''), 'bash')) : []
  const next = promptLine(props.script, promptColors, runs)
  if (next === prompt) {
    return
  }
  prompt = next
  if (shown) {
    const { output, chars } = shown
    shown = null
    term.write(FULL_RESET)
    show(output, chars)
  }
}

/**
 * Writes what `output` adds to what the terminal shows, so it keeps its
 * scrollback, and opens it with the prompt line each time it starts anew
 * (`terminalWrite`). The terminal follows the output while its view is at
 * the end, and stays where the user scrolled to otherwise.
 */
function show(output: string, chars: number | undefined): void {
  if (!term) {
    return
  }
  const { reset, text } = terminalWrite(shown, output, chars, prompt)
  shown = { output, chars }
  const written = reset ? FULL_RESET + text : text
  if (written) {
    term.write(written)
  }
}

function applyCursor(running: boolean): void {
  if (!term) {
    return
  }
  term.options.cursorBlink = running
  term.options.cursorInactiveStyle = running ? 'block' : 'none'
}

/**
 * Fits the terminal to its element once the element has a size. Without
 * one, the fit addon falls back to two columns and one row, and output
 * written that narrow wraps into the scrollback until its start falls out.
 */
function fitToHost(): void {
  const host = hostRef.value
  if (host && host.clientWidth > 0 && host.clientHeight > 0) {
    fit?.fit()
  }
}

/** Whether a change of the root's appearance attributes changed a value, not only wrote one. */
function appearanceChanged(records: readonly MutationRecord[]): boolean {
  const root = document.documentElement
  return records.some(
    (record) => record.attributeName !== null && record.oldValue !== root.getAttribute(record.attributeName),
  )
}

onMounted(() => {
  const host = hostRef.value
  if (!host) {
    return
  }
  term = new Terminal({
    convertEol: true,
    disableStdin: true,
    cursorBlink: props.running === true,
    cursorInactiveStyle: props.running ? 'block' : 'none',
    fontSize: 12,
    lineHeight: 1.4,
    // The app's code font (`styles/base.css`), which xterm takes as a family list.
    fontFamily: getComputedStyle(host).getPropertyValue('--font-mono'),
    scrollback: 2000,
  })
  fit = new FitAddon()
  term.loadAddon(fit)
  applyColors()
  term.open(host)
  fitToHost()
  show(props.output, props.chars)
  resize = new ResizeObserver(fitToHost)
  resize.observe(host)
  appearance = new MutationObserver((records) => {
    if (appearanceChanged(records)) {
      applyColors()
    }
  })
  appearance.observe(document.documentElement, {
    attributes: true,
    attributeOldValue: true,
    attributeFilter: ['data-theme', 'data-tone', 'data-accent'],
  })
})

watch(
  () => [props.output, props.chars] as const,
  ([output, chars]) => show(output, chars),
)
watch(
  () => props.running,
  (running) => applyCursor(running === true),
)
watch(terminalTheme, applyColors)
watch(() => props.script, applyPrompt)
// Bash's parser's arrival colors the script anew, and the page's mode its
// text color and its runs.
watch(useMarkdownRenderVersion(), applyColors)

onBeforeUnmount(() => {
  resize?.disconnect()
  appearance?.disconnect()
  term?.dispose()
  term = undefined
  fit = undefined
})
</script>

<template>
  <div class="h-full min-h-0 w-full px-3 py-2">
    <div
      ref="hostRef"
      class="xterm-host h-full min-h-0 w-full"
    />
  </div>
</template>

<style scoped>
.xterm-host :deep(.xterm) {
  height: 100%;
}

.xterm-host :deep(.xterm-viewport) {
  background-color: transparent !important;
  overflow-y: auto !important;
}

/* Selection decorations sit behind the glyphs so ANSI colors stay visible. */
.xterm-host :deep(.xterm-selection) {
  z-index: 0 !important;
}

.xterm-host :deep(.xterm-rows) {
  position: relative;
  z-index: 1;
}

.xterm-host :deep(.xterm-selection div) {
  background-color: var(--xterm-selection) !important;
}
</style>
