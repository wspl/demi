import { pageLog } from './log'

/**
 * The page's stutters, for the log: each run of long animation frames, the
 * frames that took the browser over 50 ms from the task that started them to
 * their paint, is one `frame.long_run` line once the run ends, with the
 * conversation it showed in and the longest frame's account of where its time
 * went. A run ends after a second with no long frame, when the page shows
 * another conversation, and when the page is hidden.
 *
 * ```text
 *   long frame --> long frame --> ... (a second with none) --> frame.long_run
 * ```
 *
 * Only Chromium's browsers report long animation frames; elsewhere the page
 * writes no such line.
 */

/** A script of a long frame, as the browser accounts for it. */
interface FrameScript {
  duration: number
  invoker: string
  sourceURL: string
  sourceFunctionName: string
  sourceCharPosition: number
  forcedStyleAndLayoutDuration: number
}

/** A long animation frame, as the browser reports it. */
interface LongFrame extends PerformanceEntry {
  blockingDuration: number
  styleAndLayoutStart: number
  firstUIEventTimestamp: number
  scripts: readonly FrameScript[]
}

/**
 * How long the page goes without a long frame before the run ends: frames
 * less than a second apart are one stutter to whoever saw it, and a line for
 * each would fill the log while a slow computer scrolls.
 */
const RUN_GAP_MS = 1_000

interface Run {
  conversation: string | null
  frames: number
  ms: number
  blocking: number
  forced: number
  input: number
  longest: LongFrame
  /** When its last frame ended. */
  end: number
}

/** Writes the page's runs of long frames to its log, `conversation` the one it shows; answers the stop. */
export function logLongFrames(conversation: () => string | null): () => void {
  if (!PerformanceObserver.supportedEntryTypes.includes('long-animation-frame')) {
    return () => {}
  }
  let run: Run | null = null
  let timer: ReturnType<typeof setTimeout> | null = null

  function end(): void {
    if (timer !== null) {
      clearTimeout(timer)
      timer = null
    }
    if (!run) {
      return
    }
    const { longest } = run
    const script = longest.scripts.reduce<FrameScript | null>((most, each) => (!most || each.duration > most.duration ? each : most), null)
    const frameEnd = longest.startTime + longest.duration
    pageLog.info('frame.long_run', {
      frames: run.frames,
      ms: Math.round(run.ms),
      blocking_ms: Math.round(run.blocking),
      forced_layout_ms: Math.round(run.forced),
      input_frames: run.input,
      longest_ms: Math.round(longest.duration),
      longest_style_layout_ms: longest.styleAndLayoutStart > 0 ? Math.round(frameEnd - longest.styleAndLayoutStart) : 0,
      ...(script
        ? {
            script_ms: Math.round(script.duration),
            script_invoker: script.invoker,
            ...(script.sourceURL
              ? { script_source: `${script.sourceFunctionName || '(anonymous)'}@${script.sourceURL.split('/').pop()}:${script.sourceCharPosition}` }
              : {}),
          }
        : {}),
    }, run.conversation ?? undefined)
    run = null
  }

  const observer = new PerformanceObserver((list) => {
    for (const frame of list.getEntries() as LongFrame[]) {
      const shown = conversation()
      // A run ends where the page goes a while without a long frame, or
      // shows another conversation.
      if (run && (run.conversation !== shown || frame.startTime - run.end > RUN_GAP_MS)) {
        end()
      }
      run ??= { conversation: shown, frames: 0, ms: 0, blocking: 0, forced: 0, input: 0, longest: frame, end: 0 }
      run.frames += 1
      run.ms += frame.duration
      run.blocking += frame.blockingDuration
      run.forced += frame.scripts.reduce((sum, script) => sum + script.forcedStyleAndLayoutDuration, 0)
      run.input += frame.firstUIEventTimestamp > 0 ? 1 : 0
      run.end = frame.startTime + frame.duration
      if (frame.duration > run.longest.duration) {
        run.longest = frame
      }
    }
    if (timer !== null) {
      clearTimeout(timer)
    }
    timer = setTimeout(end, RUN_GAP_MS)
  })
  observer.observe({ type: 'long-animation-frame', buffered: false })
  // A page being hidden or left writes its run before the log sends what it
  // holds: the capture on the window comes before the log's listeners.
  function hidden(): void {
    if (document.visibilityState === 'hidden') {
      end()
    }
  }
  globalThis.addEventListener('pagehide', end, { capture: true })
  globalThis.addEventListener('visibilitychange', hidden, { capture: true })
  return () => {
    observer.disconnect()
    globalThis.removeEventListener('pagehide', end, { capture: true })
    globalThis.removeEventListener('visibilitychange', hidden, { capture: true })
    end()
  }
}
