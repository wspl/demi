<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import {
  terminalPromptColors,
  useTerminalTheme,
  xtermThemeFromElement,
} from '../composables/useTerminalTheme'
import { promptLine, terminalWrite, type TerminalShown } from './terminals'
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

const hostRef = ref<HTMLElement | null>(null)
const { terminalTheme } = useTerminalTheme()

let term: Terminal | undefined
let fit: FitAddon | undefined
/** The output the terminal shows, and the live view's count at its end; null before anything. */
let shown: TerminalShown | null = null
/** The prompt line, in the colors of the theme it was last drawn in. */
let prompt = ''
let resize: ResizeObserver | undefined
let appearance: MutationObserver | undefined

function applyTheme(): void {
  const host = hostRef.value
  if (!term || !host) {
    return
  }
  const theme = xtermThemeFromElement(host, terminalTheme.value)
  term.options.theme = theme
  host.style.setProperty('--xterm-selection', theme.selectionBackground)
  applyPrompt()
}

/**
 * Takes the prompt line for the script in the live colors. Its colors are
 * written into the terminal, so a new theme or script draws it, and the
 * output after it, anew.
 */
function applyPrompt(): void {
  const host = hostRef.value
  if (!term || !host) {
    return
  }
  const next = promptLine(props.script, terminalPromptColors(host, terminalTheme.value))
  if (next === prompt) {
    return
  }
  prompt = next
  if (shown) {
    const { output, chars } = shown
    shown = null
    term.reset()
    show(output, chars)
  }
}

/**
 * Writes what `output` adds to what the terminal shows, so it keeps its
 * scrollback, and opens it with the prompt line each time it starts anew
 * (`terminalWrite`).
 */
function show(output: string, chars: number | undefined): void {
  if (!term) {
    return
  }
  const { reset, text } = terminalWrite(shown, output, chars, prompt)
  shown = { output, chars }
  if (reset) {
    term.reset()
  }
  if (!text) {
    return
  }
  term.write(text)
  if (props.running) {
    term.scrollToBottom()
  }
}

function applyCursor(running: boolean): void {
  if (!term) {
    return
  }
  term.options.cursorBlink = running
  term.options.cursorInactiveStyle = running ? 'block' : 'none'
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
    theme: xtermThemeFromElement(host, terminalTheme.value),
  })
  fit = new FitAddon()
  term.loadAddon(fit)
  term.open(host)
  applyTheme()
  fit.fit()
  requestAnimationFrame(() => {
    fit?.fit()
    requestAnimationFrame(() => fit?.fit())
  })
  show(props.output, props.chars)
  resize = new ResizeObserver(() => fit?.fit())
  resize.observe(host)
  appearance = new MutationObserver(applyTheme)
  appearance.observe(document.documentElement, {
    attributes: true,
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
watch(terminalTheme, applyTheme)
watch(() => props.script, applyPrompt)

onBeforeUnmount(() => {
  resize?.disconnect()
  appearance?.disconnect()
  term?.dispose()
  term = undefined
  fit = undefined
})
</script>

<template>
  <div
    ref="hostRef"
    class="xterm-host h-full min-h-0 w-full px-3 py-2"
  />
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
