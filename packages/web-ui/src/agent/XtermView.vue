<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import {
  useTerminalTheme,
  xtermThemeFromElement,
} from '../composables/useTerminalTheme'
import { liveOutputDelta } from './terminals'
import '@xterm/xterm/css/xterm.css'

const MONO = 'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace'

const props = defineProps<{
  output: string
  /**
   * The live view's count of characters at the end of `output`; absent for
   * output that is whole, such as a transcript's.
   */
  chars?: number
  running?: boolean
}>()

const hostRef = ref<HTMLElement | null>(null)
const { terminalTheme } = useTerminalTheme()

let term: Terminal | undefined
let fit: FitAddon | undefined
/** The output the terminal shows, and the live view's count at its end. */
let written = ''
let shownChars: number | undefined
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
}

/** What whole `output` adds to the output written: its rest, or all of it anew. */
function continuation(output: string): { anew: boolean; text: string } {
  return output.startsWith(written)
    ? { anew: false, text: output.slice(written.length) }
    : { anew: true, text: output }
}

/**
 * Writes what `output` adds to what the terminal shows, so it keeps its
 * scrollback: for a live view, the characters beyond those shown, by the
 * view's count (`runtime.md` § Rendering boundary); for whole output, what
 * continues it. Anything else is shown anew.
 */
function show(output: string, chars: number | undefined): void {
  if (!term) {
    return
  }
  const { anew, text } = chars === undefined
    ? continuation(output)
    : liveOutputDelta(shownChars, output, chars)
  written = output
  shownChars = chars
  if (anew) {
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
    fontFamily: MONO,
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
