import { ref } from 'vue'
import { DOCK_WINDOW_SHARE } from '@demicodes/web-ui/agent/dock-window'

/**
 * The share of the session area's height the dock windows keep, one for every
 * specimen: the product keeps it in the account's local preferences, so the
 * windows here keep one between them, for as long as the page stays open.
 */
export const dockWindowShare = ref<number>(DOCK_WINDOW_SHARE.default)
