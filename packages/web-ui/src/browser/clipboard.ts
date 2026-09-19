/**
 * The viewer's own clipboard (`browser-live-view.md` § Input). A copy in the
 * watched tab is the viewer's copy: the page starts the write while the
 * keystroke still carries the viewer's activation, and finishes it with the
 * text the Host sends back.
 */
const WAIT_MS = 3000

export class ViewerClipboard {
  private pending: { resolve: (text: string) => void; fail: () => void } | null = null

  /** The viewer pressed copy or cut in the watched tab. */
  expect(now = () => Date.now()): void {
    if (typeof ClipboardItem !== 'function' || !navigator.clipboard?.write) {
      return
    }
    this.pending?.fail()
    let settle: { resolve: (text: string) => void; fail: () => void }
    const text = new Promise<Blob>((resolve, reject) => {
      settle = {
        resolve: (value) => resolve(new Blob([value], { type: 'text/plain' })),
        fail: () => reject(new Error('the tab copied nothing')),
      }
    })
    const pending = settle!
    this.pending = pending
    const timer = setTimeout(() => pending.fail(), WAIT_MS)
    void text.catch(() => {}).finally(() => {
      clearTimeout(timer)
      if (this.pending === pending) {
        this.pending = null
      }
    })
    void navigator.clipboard.write([new ClipboardItem({ 'text/plain': text })]).catch(() => {})
    void now
  }

  /** Text the watched tab copied. */
  receive(text: string): void {
    if (this.pending) {
      this.pending.resolve(text)
      this.pending = null
      return
    }
    void navigator.clipboard?.writeText?.(text).catch(() => {})
  }
}

/** One viewer, one clipboard. */
export const viewerClipboard = new ViewerClipboard()
