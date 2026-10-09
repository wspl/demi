// A browser that stops answering (browse.md § What `demi` adds): while a
// call runs, the browser is asked a question of its own every so often,
// which it answers at once unless its process hangs. A script waits on the
// browser in ways no timeout of Playwright's covers (a key press, a
// protocol command, an evaluation), so whatever the script waits for, the
// call stops waiting once the browser leaves that question unanswered.
import type { Browser } from './browser'

/** What `HangWatch.within` answers when the browser stopped answering first. */
export const HUNG = Symbol('hung')

/** How long the browser may leave a question unanswered before the call counts it as hanging. */
export const HANG_MS = 5_000

export class HangWatch {
  private timer: ReturnType<typeof setTimeout> | undefined
  private stopped = false
  private readonly hang = Promise.withResolvers<typeof HUNG>()

  /** Starts asking `browser`, every fifth of `ms`, whether it answers within `ms`. */
  constructor(
    private readonly browser: Browser,
    private readonly ms: number,
  ) {
    this.next()
  }

  /** `work`'s result, or HUNG when the browser stops answering first. */
  within<T>(work: Promise<T>): Promise<T | typeof HUNG> {
    // Once the browser hung first, the work's failure has no one to report
    // to: the call already says that the browser did not answer.
    work.catch(() => undefined)
    return Promise.race([work, this.hang.promise])
  }

  /** Stops asking; a question in flight ends within `ms` and asks no other. */
  stop(): void {
    this.stopped = true
    clearTimeout(this.timer)
  }

  private next(): void {
    if (!this.stopped) {
      this.timer = setTimeout(() => void this.ask(), this.ms / 5)
    }
  }

  private async ask(): Promise<void> {
    if (await this.browser.answers(this.ms)) {
      this.next()
    } else {
      this.hang.resolve(HUNG)
    }
  }
}
