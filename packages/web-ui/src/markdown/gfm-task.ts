const FENCE = /^ {0,3}(`{3,}|~{3,})/
const TASK_LINE = /^((?:>[ \t]?)*[ \t]*(?:[-+*]|\d+[.)])[ \t]+)\[([ xX])\](?=\s|$)/

/**
 * Flips the nth rendered task box in the source. Counts what marked renders as a task item:
 * list items, including blockquoted ones, and never a line inside a fenced code block.
 */
export function toggleGfmTask(src: string, index: number): string {
  let seen = -1
  let fence: string | null = null
  return src.split('\n').map((line) => {
    const fenceMatch = FENCE.exec(line)
    if (fenceMatch) {
      const marker = fenceMatch[1]!
      if (!fence) fence = marker
      else if (marker[0] === fence[0] && marker.length >= fence.length) fence = null
      return line
    }
    if (fence) return line
    const match = TASK_LINE.exec(line)
    if (!match) return line
    seen += 1
    if (seen !== index) return line
    return `${match[1]}[${match[2] === ' ' ? 'x' : ' '}]${line.slice(match[0].length)}`
  }).join('\n')
}

export function liveCheckboxHtml(checked: boolean): string {
  return `<input ${checked ? 'checked="" ' : ''}type="checkbox"> `
}
