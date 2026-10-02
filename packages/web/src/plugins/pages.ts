import { browserPage } from '@demicodes/plugin-browser'
import { exposePage } from '@demicodes/plugin-expose'
import { skillsPage } from '@demicodes/plugin-skills'
import type { PluginPage } from '@demicodes/web-ui/plugins/slots'

/** The plugin packages the product shows, a static list (`plugins.md` § The page). */
export const PLUGIN_PAGES: readonly PluginPage[] = [skillsPage, browserPage(), exposePage]
