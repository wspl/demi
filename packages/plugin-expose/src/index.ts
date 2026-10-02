// The `expose` plugin's page (`plugin-pages.md`, `expose.md` § Product
// surface): its conversation header tool, the `page` work panel kind its
// exposes open in, and the types of its state, parameters and results,
// generated from the plugin's Rust types.
import type { PluginPage } from '@demicodes/plugin-sdk'
import ExposeTool from './ExposeTool.vue'
import { pageTabKind } from './page/page'

export * from './generated/plugin'

export const exposePage: PluginPage = {
  plugin: 'expose',
  headerTool: ExposeTool,
  panelKinds: () => ({ kinds: [pageTabKind] }),
}

/** The page the registry imports (`plugin-pages.md` § Registration). */
export default exposePage
