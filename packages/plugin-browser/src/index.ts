// The `browser` plugin's page (`plugin-pages.md`, `live-view.md` § A
// browser tab in the panel): its `browser` work panel kind with the live
// view, and the types of its state, parameters, results and stream,
// generated from the plugin's Rust types.
import { browserTabKind } from './live/kind'
import {
  BrowserTabsController,
  browserTabDataSchema,
  type BrowserTabData,
  type BrowserTabsOptions,
} from './live/tabs'
import type { PluginPage } from '@demicodes/plugin-sdk'
import { browserTabsApi } from './tabs'

export * from './generated/plugin'
export { browserTabsApi } from './tabs'

/**
 * The page, whose controllers take `options`: the page's visibility and the
 * pictures' support, the document's own unless the gallery gives them.
 */
export function browserPage(options: BrowserTabsOptions = {}): PluginPage {
  return {
    plugin: 'browser',
    panelKinds: ({ plugin, tabs }) => {
      const controller = new BrowserTabsController(
        browserTabsApi(plugin),
        {
          bound: () => {
            const bound: BrowserTabData[] = []
            for (const data of tabs.bound('browser')) {
              const parsed = browserTabDataSchema.safeParse(data)
              // A tab whose data does not fit the kind binds nothing.
              if (parsed.success) {
                bound.push(parsed.data)
              }
            }
            return bound
          },
          add: (data) => tabs.add('browser', data),
        },
        options,
      )
      void controller.refresh()
      return {
        kinds: [browserTabKind(controller)],
        refresh: () => void controller.refresh(),
        dispose: () => controller.dispose(),
      }
    },
  }
}

/** The page the registry imports (`plugin-pages.md` § Registration). */
export default browserPage()
