/**
 * The plugin page API (`plugin-pages.md`): everything a plugin page may use
 * of the web app. A page imports this package, `@demicodes/utils`, Vue, Zod
 * and its own generated types, and nothing else of the workspace. Its major
 * version is the page API's version: removing or changing an export
 * incompatibly raises it.
 */

// The slots a page fills.
export type {
  PanelKinds,
  PanelKindsContext,
  PluginHeaderToolProps,
  PluginPage,
  PluginSettingsSection,
} from '@demicodes/web-ui/plugins/slots'
export type { PanelTabKind } from '@demicodes/web-ui/agent/panel-kinds/kind'

// The plugin client.
export {
  PluginCallError,
  usePlugin,
  type ConversationPluginClient,
  type PluginCallOptions,
  type PluginClient,
} from '@demicodes/web-ui/plugins/client'
export type {
  OpenUserStream,
  StreamBytes,
  UserStream,
  UserStreamHandlers,
} from '@demicodes/web-ui/plugins/streams'
export type { HostInstall } from '@demicodes/web-ui/devices/installs'

// Services.
export { reportError } from '@demicodes/web-ui/infra/errors'
export { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
export type { OverlayStore } from '@demicodes/web-ui/overlay/overlayStore'
export {
  pageReturned,
  waitToReconnect,
  watchSilence,
  type ReconnectWait,
  type SilenceWatch,
} from '@demicodes/web-ui/transport/liveness'
export { formatTimeRemaining, useTimeRemaining } from '@demicodes/web-ui/composables/useRelativeTime'

// The public components.
export { default as AddressBar } from '@demicodes/web-ui/agent/AddressBar.vue'
export { default as Button } from '@demicodes/web-ui/ui/Button.vue'
export { default as Dialog } from '@demicodes/web-ui/ui/Dialog.vue'
export { default as Dropdown } from '@demicodes/web-ui/ui/Dropdown.vue'
export { default as ExternalLink } from '@demicodes/web-ui/ui/ExternalLink.vue'
export { default as Fold } from '@demicodes/web-ui/ui/Fold.vue'
export { default as FoldChevron } from '@demicodes/web-ui/ui/FoldChevron.vue'
export { default as HostInstalls } from '@demicodes/web-ui/devices/HostInstalls.vue'
export { default as IconButton } from '@demicodes/web-ui/ui/IconButton.vue'
export { default as IndeterminateSpinner } from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
export { default as Menu } from '@demicodes/web-ui/ui/Menu.vue'
export { default as MenuDivider } from '@demicodes/web-ui/ui/MenuDivider.vue'
export { default as MenuGroup } from '@demicodes/web-ui/ui/MenuGroup.vue'
export { default as MenuItem } from '@demicodes/web-ui/ui/MenuItem.vue'
export { default as Popover } from '@demicodes/web-ui/ui/Popover.vue'
export { default as SettingsGroup } from '@demicodes/web-ui/settings/SettingsGroup.vue'
export { default as SettingsPage } from '@demicodes/web-ui/settings/SettingsPage.vue'
export { default as SettingsRow } from '@demicodes/web-ui/settings/SettingsRow.vue'
export { default as Switch } from '@demicodes/web-ui/ui/Switch.vue'
export { default as TextInput } from '@demicodes/web-ui/ui/TextInput.vue'
export { default as Tooltip } from '@demicodes/web-ui/ui/Tooltip.vue'
export { GlobePlus } from '@demicodes/web-ui/ui/GlobePlus'
export { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
