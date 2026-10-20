// `demi.frames(probe)`: runs `probe`, a function of the page's, at every frame
// the page draws from now on, and `stop()` answers what it read in each, with
// when.
//
// It serves what a single frame shows that a picture cannot prove, such as a list
// that never draws a frame shorter than the one before or a row that keeps its
// key. The probe runs in the page's animation-frame callback, before the frame is
// painted, on the document that frame paints.
import { Failure, type Tool } from '../tool'

/** What the probe read in a frame, at the frame's time in the page's milliseconds since the recording began. */
export interface FrameReading<T> {
  at: number
  value: T
}

export interface FrameRecording<T> {
  stop: () => Promise<FrameReading<T>[]>
}

let recordings = 0

export async function frames<T>(tool: Tool, probe: () => T): Promise<FrameRecording<T>> {
  if (typeof probe !== 'function') {
    throw new Failure('demi.frames takes a function to run in the page at every frame, such as demi.frames(() => document.body.scrollHeight)')
  }
  const page = await tool.browser.page()
  const name = `__demiFrames${++recordings}`
  await page.evaluate(([key, source]) => {
    // The probe comes as its source, since a function cannot cross into the page.
    const read = (0, eval)(`(${source})`) as () => unknown
    const readings: FrameReading<unknown>[] = []
    const started = performance.now()
    const state = { readings, frame: 0 }
    const tick = (now: number) => {
      readings.push({ at: Math.round(now - started), value: read() })
      state.frame = requestAnimationFrame(tick)
    }
    state.frame = requestAnimationFrame(tick)
    Object.assign(window, { [key]: state })
  }, [name, probe.toString()] as const)
  return {
    stop: () => page.evaluate((key) => {
      const state: { readings: FrameReading<T>[], frame: number } | undefined = Reflect.get(window, key)
      if (!state) {
        return []
      }
      cancelAnimationFrame(state.frame)
      Reflect.deleteProperty(window, key)
      return state.readings
    }, name),
  }
}
