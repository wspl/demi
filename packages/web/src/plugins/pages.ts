import { browserPage } from '@demicodes/plugin-browser'
import { changesPage } from '@demicodes/plugin-changes'
import { exposePage } from '@demicodes/plugin-expose'
import { fileBrowserPage } from '@demicodes/plugin-file-browser'
import { skillsPage } from '@demicodes/plugin-skills'
import type { PluginPage } from '@demicodes/web-ui/plugins/slots'

/** The plugin packages the product shows, a static list (`plugin-pages.md` § Registration). */
export const PLUGIN_PAGES: readonly PluginPage[] = [skillsPage, browserPage(), exposePage, changesPage, fileBrowserPage]
