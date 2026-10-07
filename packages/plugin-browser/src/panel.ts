/**
 * The browser plugin's panel session of one conversation: the tabs of the
 * agent's browser, whose content is the live view, and the tabs of the
 * user's browser, the web previews (`preview.md` § What the user sees).
 */
import type { PanelSession } from '@demicodes/plugin-sdk'
import type { BrowserTabsController } from './live/tabs'
import type { PreviewTabs } from './preview/tabs'

export class BrowserPanel implements PanelSession {
  constructor(
    readonly browser: BrowserTabsController,
    readonly preview: PreviewTabs,
    /** Drops the calls the previews still wait for, as the session ends. */
    private readonly calls: AbortController = new AbortController(),
  ) {}

  dispose(): void {
    this.calls.abort()
    this.browser.dispose()
    this.preview.dispose()
  }
}
