import process from 'node:process'

export interface ReplOutput {
  write(text: string): void
  isTTY?: boolean
}

export type Tone = 'red' | 'green' | 'yellow' | 'blue' | 'cyan' | 'dim' | 'bold'

export function color(text: string, tone: Tone, output: ReplOutput = process.stdout): string {
  if (!output.isTTY) return text
  const codes: Record<Tone, [number, number]> = {
    red: [31, 39],
    green: [32, 39],
    yellow: [33, 39],
    blue: [34, 39],
    cyan: [36, 39],
    dim: [2, 22],
    bold: [1, 22],
  }
  const [open, close] = codes[tone]
  return `\x1b[${open}m${text}\x1b[${close}m`
}

export function writeLine(text = ''): void {
  process.stdout.write(`${text}\n`)
}

export function writeLineTo(output: ReplOutput, text = ''): void {
  output.write(`${text}\n`)
}

export function writeMetaLine(label: string, value: string): void {
  writeLine(`${label.padEnd(10)}${value}`)
}

export function writePrefixed(output: ReplOutput, label: string, text: string, tone: Tone): void {
  const lines = text.endsWith('\n') ? text.slice(0, -1).split('\n') : text.split('\n')
  for (const line of lines) writeLineTo(output, `${color(`${label}>`, tone, output)} ${line}`)
}

export function writeEventLine(output: ReplOutput, label: string, text: string, tone: Tone): void {
  writeLineTo(output, `${color(`${label}>`, tone, output)} ${text}`)
}
