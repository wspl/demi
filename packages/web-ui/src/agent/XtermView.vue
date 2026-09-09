<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import {
  useTerminalTheme,
  xtermThemeFromElement,
} from '../composables/useTerminalTheme'
import '@xterm/xterm/css/xterm.css'

const MONO = 'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace'

const props = defineProps<{
  output: string
  running?: boolean
}>()

const hostRef = ref<HTMLElement | null>(null)
const { terminalTheme } = useTerminalTheme()

let term: Terminal | undefined
let fit: FitAddon | undefined
let written = ''
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

function writeDelta(next: string): void {
  if (!term) {
    return
  }
  if (!next.startsWith(written)) {
    term.reset()
    written = ''
  }
  const add = next.slice(written.length)
  if (!add) {
    return
  }
  term.write(add)
  written = next
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
  writeDelta(props.output)
  resize = new ResizeObserver(() => fit?.fit())
  resize.observe(host)
  appearance = new MutationObserver(applyTheme)
  appearance.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-theme', 'data-tone', 'data-accent'],
  })
})

watch(() => props.output, writeDelta)
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
