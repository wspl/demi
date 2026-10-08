import { onBeforeUnmount } from 'vue'
import { followLiveOutput, type TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import { DEMO_WATCH_BURST, DEMO_WATCH_LINES } from './fixtures/terminal-output'
import { LIVE_TERMINAL_ID } from './fixtures/terminals'

/** How often the product's backend sends a command's new output, at most (`runtime.md` § Live output). */
const FRAME_MS = 250
/** The characters of the pages' view a frame carries. */
const TAIL_CHARS = 4096
/** Every this many frames, a burst longer than a frame's tail. */
const BURST_EVERY = 48

/**
 * Stands in for the backend: `terminal` receives the frame the product's
 * page would after the command printed `text`, the pages' view's last
 * 4,096 characters and their count, and web-ui's rule adds it to what the
 * page shows.
 */
export function printLive(terminal: TerminalRecord, text: string): void {
  const chars = (terminal.chars ?? 0) + Array.from(text).length
  const tail = Array.from(terminal.output + text).slice(-TAIL_CHARS).join('')
  terminal.output = followLiveOutput(terminal.output, terminal.chars, tail, chars)
  terminal.chars = chars
}

/**
 * Plays the gallery's live command among `terminals`: a line a frame, and
 * now and then a burst longer than a frame's tail, which the page shows
 * anew. It prints nothing while the command is not running, as after a
 * page's stop, until the gallery runs it again, and stops when the component
 * unmounts.
 */
export function useLiveGalleryCommand(terminals: readonly TerminalRecord[]): void {
  let frame = 0
  const timer = window.setInterval(() => {
    const terminal = terminals.find((entry) => entry.id === LIVE_TERMINAL_ID)
    if (terminal?.phase !== 'running') {
      return
    }
    frame += 1
    const text = frame % BURST_EVERY === 0
      ? DEMO_WATCH_BURST
      : DEMO_WATCH_LINES[frame % DEMO_WATCH_LINES.length]!
    printLive(terminal, `${text}\n`)
  }, FRAME_MS)
  onBeforeUnmount(() => window.clearInterval(timer))
}
