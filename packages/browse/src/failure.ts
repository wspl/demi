// How a call reports the error its script stopped at (browse.md § A call,
// Failures): the script's line and its text, then what went wrong, trimmed
// to what the agent needs to fix the script. Playwright's own message says
// what it looked for and found: its strict mode lists every element a
// locator matched, and its call log names what covers an element an action
// waited for; the call log's retries and scrolling are left out. The tool's
// own stack frames are never shown.
import { stripVTControlCharacters } from 'node:util'
import { buildMessageOf } from './script'
import { Failure } from './tool'

/** Where the script stopped: the line, its text, and the file the caller named, null for standard input. */
export interface Where {
  line: number | null
  text: string
  file: string | null
}

/** How many of the call log's steps a failure keeps, the last ones. */
const STEPS = 6

/** Steps of Playwright's call log that only say it retried or scrolled. */
const NOISE = [
  /^attempting \w+ action$/,
  /^retrying \w+ action$/,
  /^waiting \d+ms$/,
  /^waiting for element to be visible, enabled and stable$/,
  /^element is visible, enabled and stable$/,
  /^scrolling into view if needed$/,
  /^done scrolling$/,
  // An assertion's first step repeats its head, which names the matcher and the timeout.
  /^Expect "/,
]

/** The lines that report `error`, which the script threw at `where`. */
export function describeFailure(error: unknown, where: Where): string[] {
  let at = 'Failed'
  if (where.line !== null) {
    const place = where.file === null ? `line ${where.line}` : `${where.file}:${where.line}`
    at = `Failed at ${place}: ${where.text}`
  }
  return [at, ...messageOf(error).map((line) => `  ${line}`)]
}

/** What went wrong, in lines. */
function messageOf(error: unknown): string[] {
  const compiled = buildMessageOf(error)
  if (compiled) {
    return [`The script does not compile: ${compiled.message}`]
  }
  if (error instanceof Failure) {
    return error.message.split('\n')
  }
  if (!(error instanceof Error)) {
    return [`The script threw ${String(error)}`]
  }
  const message = stripVTControlCharacters(error.message)
  if (/\nCall log:\n/.test(message)) {
    return trimPlaywright(message)
  }
  // An error of the script's own, such as a TypeError, is named by its kind.
  return `${error.name}: ${message}`.split('\n')
}

/**
 * Playwright's message without its noise: the head as Playwright wrote it,
 * less the `Error: ` after the action's name, then the distinct steps of its
 * call log that say what it found, the last ones.
 */
export function trimPlaywright(message: string): string[] {
  const [head, log = ''] = message.split(/\n+Call log:\n/)
  const lines = head.split('\n')
    .filter((line) => line.trim() !== '')
    // An element strict mode lists is named by its tag and the locator that
    // finds it alone, not by its attributes, which are mostly classes.
    .map((line) => line.replace(/^(\s*\d+\) )<([\w-]+)[^>]*>.*? aka (.+)$/, '$1<$2> aka $3'))
  lines[0] = lines[0].replace(/^([\w.]+): Error: /, '$1: ')
  const steps: string[] = []
  for (const raw of log.split('\n')) {
    const step = raw.trim().replace(/^- /, '').replace(/^\d+ × /, '')
    if (step === '' || NOISE.some((noise) => noise.test(step)) || steps.includes(step)) {
      continue
    }
    steps.push(step)
  }
  const kept = steps.slice(-STEPS)
  return [...lines, ...steps.length > kept.length ? ['…'] : [], ...kept]
}
