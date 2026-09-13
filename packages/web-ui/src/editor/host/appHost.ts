import { appThemeStore } from '../../theme/appTheme'
import { t } from '../../infra/i18n'
import { renderMarkdown } from '../../markdown/render'
import { createEditorHostDefaults } from './defaults'
import type { EditorHost } from './types'

/**
 * The editor host for this app: the app's theme store for mode and code
 * theme, its strings and its markdown renderer. Language services stay the
 * no-op defaults until an LSP transport exists.
 */
export function createAppEditorHost(): EditorHost {
  const defaults = createEditorHostDefaults()
  return {
    ...defaults,
    theme: {
      getSnapshot: () => ({
        mode: appThemeStore.state.mode,
        codeThemeId: appThemeStore.state.codeThemeId,
        palette: null,
      }),
      subscribe: appThemeStore.subscribe,
    },
    i18n: { t },
    markdown: {
      render: (src, options) => renderMarkdown(src, options),
    },
  }
}

export const appEditorHost: EditorHost = createAppEditorHost()
