// The `widget` plugin's page (`packages/plugin-sdk/src/index.ts`): the
// widget a reply's line `<widget id="w1" />` shows in place, and the work
// panel's tab that shows one whole.
import { definePage } from '@demicodes/plugin-sdk'
import { PLUGIN } from './generated/plugin'
import WidgetBlock from './WidgetBlock.vue'
import { widgetKind } from './tab'

export const widgetPage = definePage({ plugin: PLUGIN, kinds: [widgetKind], blocks: [{ tag: 'widget', component: WidgetBlock }] })

/** The page the registry imports (`crates/xtask/src/contracts/registry.rs`). */
export default widgetPage
