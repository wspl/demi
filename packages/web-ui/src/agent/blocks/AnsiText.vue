<script setup lang="ts">
import { computed, toRef } from 'vue'
import { refThrottled } from '@vueuse/core'
import AnsiToHtml from 'ansi-to-html'
import { useTerminalTheme } from '@demicodes/web-ui/composables/useTerminalTheme'

const RENDER_THROTTLE = 80

const props = defineProps<{
  content: string
}>()

const throttledContent = refThrottled(toRef(props, 'content'), RENDER_THROTTLE)

const { terminalTheme } = useTerminalTheme()

const ansiConverter = computed(() => {
  const t = terminalTheme.value
  return new AnsiToHtml({
    fg: t.foreground,
    bg: t.background,
    escapeXML: true,
    colors: {
      0: t.black,
      1: t.red,
      2: t.green,
      3: t.yellow,
      4: t.blue,
      5: t.magenta,
      6: t.cyan,
      7: t.white,
      8: t.brightBlack,
      9: t.brightRed,
      10: t.brightGreen,
      11: t.brightYellow,
      12: t.brightBlue,
      13: t.brightMagenta,
      14: t.brightCyan,
      15: t.brightWhite,
    } as Record<number, string>,
  })
})

function normalizeTerminalContent(content: string): string {
  if (!content.includes('\r')) return content

  const lines: string[] = []
  let currentLine = ''

  for (const char of content) {
    if (char === '\r') {
      currentLine = ''
      continue
    }
    if (char === '\n') {
      lines.push(currentLine)
      currentLine = ''
      continue
    }
    currentLine += char
  }

  lines.push(currentLine)
  return lines.join('\n')
}

const renderedHtml = computed(() => {
  if (!throttledContent.value) return ''
  return ansiConverter.value.toHtml(normalizeTerminalContent(throttledContent.value))
})
</script>

<template>
  <pre class="font-mono text-[13px] leading-[1.4] text-fg-body whitespace-pre-wrap break-words" v-html="renderedHtml" />
</template>
