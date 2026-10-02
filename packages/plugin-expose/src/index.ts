// The `expose` plugin's page (`plugin-pages.md`, `expose.md` § Product
// surface): its conversation header tool, the `page` work panel kind its
// exposes open in, and the types of its state, parameters and results,
// generated from the plugin's manifest.
import { definePage } from '@demicodes/plugin-sdk'
import { PLUGIN } from './generated/plugin'
import ExposeTool from './ExposeTool.vue'
import { pageTabKind } from './page/page'

export * from './generated/plugin'

export const exposePage = definePage({
  plugin: PLUGIN,
  headerTool: ExposeTool,
  kinds: [pageTabKind],
})

/** The page the registry imports (`plugin-pages.md` § Registration). */
export default exposePage
