import { browserPage } from '@demicodes/plugin-browser'
import { exposePage } from '@demicodes/plugin-expose'
import { skillsPage } from '@demicodes/plugin-skills'
import type { PluginPage } from '@demicodes/web-ui/plugins/slots'

/**
 * The plugin packages the gallery shows, a static list as the product's
 * (`plugins.md` § The page); a specimen that needs the browser's page with
 * options of its own makes one.
 */
export const GALLERY_PLUGIN_PAGES: readonly PluginPage[] = [skillsPage, browserPage(), exposePage]
