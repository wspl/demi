/**
 * The plugin page API (`plugin-pages.md`): everything a plugin page may use
 * of the web app. A page imports this package, `@demicodes/utils`, Vue, Zod
 * and its own generated types, and nothing else of the workspace. Its major
 * version is the page API's version: removing or changing an export
 * incompatibly raises it.
 */

// The page object and its context (`plugin-pages.md` § The page object,
// § The page context).
export {
  PluginCallError,
  definePage,
  type AnyPluginPage,
  usePage,
  type ConversationFileService,
  type ConversationPlugin,
  type IntentService,
  type KindTab,
  type PageContext,
  type PagePlugin,
  type PanelKind,
  type PanelSession,
  type PluginCallOptions,
  type PluginPage,
  type PluginSettingsSection,
  type PluginState,
} from '@demicodes/web-ui/plugins/page'
export { pendingCalls, type PendingCalls } from '@demicodes/web-ui/plugins/calls'
export type { IntentName, IntentPayloads, IntentRequest } from '@demicodes/web-ui/plugins/intents'
export type {
  OpenUserStream,
  StreamBytes,
  UserStream,
  UserStreamHandlers,
} from '@demicodes/web-ui/plugins/streams'
export type { HostInstall } from '@demicodes/web-ui/devices/installs'
export type { OverlayStore } from '@demicodes/web-ui/overlay/overlayStore'
export type { SettingsRowStatus } from '@demicodes/web-ui/settings/types'
// The style a plugin's own prop of UI text declares (the gallery's Writing page).
export type { HeadlineText, PlaceholderText, SentenceText, TitleText } from '@demicodes/web-ui/ui/ui-text'

// The plugin kit (`plugin-pages.md` § The plugin kit).
// Streams: the liveness a stream's protocol uses.
export {
  pageReturned,
  waitToReconnect,
  watchSilence,
  type ReconnectWait,
  type SilenceWatch,
} from '@demicodes/web-ui/transport/liveness'
// Composables.
export { formatTimeRemaining, useTimeRemaining } from '@demicodes/web-ui/composables/useRelativeTime'

// Files: the shapes the conversation files service gives, and their paths.
export {
  callChangeSource,
  callEditSelectionSchema,
  emptyChangeSet,
  type CallEditSelection,
  type ChangeFile,
  type ChangeMode,
  type ChangeSetSource,
  type ChangeSources,
  type ReadCallChange,
} from '@demicodes/web-ui/files/changes'
export type { FileBrowserSource } from '@demicodes/web-ui/files/types'
export { baseName, joinPath, relativePath } from '@demicodes/web-ui/files/paths'
export { treeLayout } from '@demicodes/web-ui/files/tree-layout'

// Components: settings, controls, progress, navigation, files and icons.
export { default as AddressBar } from '@demicodes/web-ui/agent/AddressBar.vue'
export { default as Button } from '@demicodes/web-ui/ui/Button.vue'
export { default as ChangeView } from '@demicodes/web-ui/files/ChangeView.vue'
export { default as Dialog } from '@demicodes/web-ui/ui/Dialog.vue'
export { default as Dropdown } from '@demicodes/web-ui/ui/Dropdown.vue'
export { default as ExternalLink } from '@demicodes/web-ui/ui/ExternalLink.vue'
export { default as Fold } from '@demicodes/web-ui/ui/Fold.vue'
export { default as FileIcon } from '@demicodes/web-ui/files/FileIcon.vue'
export { default as FileView } from '@demicodes/web-ui/files/FileView.vue'
export { default as FoldChevron } from '@demicodes/web-ui/ui/FoldChevron.vue'
export { default as HostInstalls } from '@demicodes/web-ui/devices/HostInstalls.vue'
export { default as IconButton } from '@demicodes/web-ui/ui/IconButton.vue'
export { default as IndeterminateSpinner } from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
export { default as Menu } from '@demicodes/web-ui/ui/Menu.vue'
export { default as MenuDivider } from '@demicodes/web-ui/ui/MenuDivider.vue'
export { default as MenuGroup } from '@demicodes/web-ui/ui/MenuGroup.vue'
export { default as MenuItem } from '@demicodes/web-ui/ui/MenuItem.vue'
export { default as Popover } from '@demicodes/web-ui/ui/Popover.vue'
export { default as ProgressLine } from '@demicodes/web-ui/ui/ProgressLine.vue'
export { default as SettingsGroup } from '@demicodes/web-ui/settings/SettingsGroup.vue'
export { default as SettingsPage } from '@demicodes/web-ui/settings/SettingsPage.vue'
export { default as SettingsRow } from '@demicodes/web-ui/settings/SettingsRow.vue'
export { default as Switch } from '@demicodes/web-ui/ui/Switch.vue'
export { default as TextInput } from '@demicodes/web-ui/ui/TextInput.vue'
export { default as Tooltip } from '@demicodes/web-ui/ui/Tooltip.vue'
export { GlobePlus } from '@demicodes/web-ui/ui/GlobePlus'
export { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
