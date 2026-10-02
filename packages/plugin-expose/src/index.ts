// The `expose` plugin's page (`plugins.md` § The page, `expose.md`
// § Product surface): its conversation header tool, and the types of its
// state, parameters and results, generated from the plugin's Rust types.
import type { PluginPage } from '@demicodes/web-ui/plugins/slots'
import ExposeTool from './ExposeTool.vue'

export * from './generated/plugin'

export const exposePage: PluginPage = {
  plugin: 'expose',
  headerTool: ExposeTool,
}
