/**
 * The plugin page API: everything a plugin page may use of the web app.
 *
 * The web app is a shell that plugins fill, and a page is written only against
 * this package. The shell therefore knows no plugin, and a plugin knows no part
 * of the shell beyond this package. A page imports this package,
 * `@demicodes/utils`, Vue, Zod and its own generated types, and nothing else of
 * the workspace.
 *
 * ```text
 * web (services over the backend)      web-gallery (fixture services)
 *            \                              /
 *             +--> generated registry <----+
 *                        |
 *          plugin-browser  plugin-skills  plugin-changes  ...  (feature UI)
 *                        |
 *                   plugin-sdk   definePage, usePage, intents, services, kit
 *                        |
 *                     web-ui     the shell's components and the primitives
 * ```
 *
 * A page is a plain object, `definePage`, that names its plugin and says what
 * it contributes. Everything else about it derives from the plugin's manifest
 * (`crates/plugin-interface/src/manifest.rs`): its plugin's id, imported from
 * its generated module rather than spelled, its types, and its place in the
 * registry. Its fields are fixed: a page declares what it fills, not what it
 * does at a given moment. A slot joins the SDK when a feature needs it, not
 * before.
 *
 * The kit's components are chosen as a set, not by what one page happened to
 * use. A primitive joins it when a page needs it and another page could use it;
 * one only a single page could use stays in that page.
 *
 * The package's major version is the page API's version:
 *
 * - A change that could break a page written against it raises it, such as a
 *   slot, a part of the page context, an intent or a kit component removed or
 *   changed incompatibly. Adding one does not.
 * - Pages of this repository always build against the SDK beside them.
 *
 * The package is private until pages from outside the repository load, which is
 * open and decided with a TypeScript SDK for plugins. The design keeps it
 * possible: such a page is an ES module built against one SDK major version,
 * with Vue and the SDK supplied by the page around it, and every part of the
 * page context passes data. A page whose code nobody reviewed can therefore run
 * in a sandboxed frame, with `usePage()` over messages to the shell, without
 * changing the page's code.
 */

// The page object and its context.
export {
  PluginCallError,
  type PluginCallAnswer,
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
export type { IntentName, IntentPayloads, IntentRequest } from '@demicodes/web-ui/plugins/intents'
export type {
  OpenUserStream,
  StreamBytes,
  UserStream,
  UserStreamHandlers,
} from '@demicodes/web-ui/plugins/streams'
export type { HostArtifact } from '@demicodes/web-ui/devices/installed'
export type { OverlayStore } from '@demicodes/web-ui/overlay/overlayStore'
// What the page says of a failure, in the page's language.
export { errorMessage } from '@demicodes/web-ui/infra/errors'
export type { SettingsRowStatus } from '@demicodes/web-ui/settings/types'
// The style a plugin's own prop of UI text declares (the gallery's Writing page).
export type { HeadlineText, PlaceholderText, SentenceText, TitleText } from '@demicodes/web-ui/ui/ui-text'

// The plugin's own text in every language Demi speaks:
// `definePluginText(PLUGIN, { en, ... })` in its `src/locales/text.ts`.
export { definePluginText, type Catalogue, type Text } from '@demicodes/web-ui/i18n/text'
export type { Language } from '@demicodes/web-ui/i18n/languages'

// The plugin kit. Streams: the liveness a stream's protocol uses.
export {
  pageReturned,
  pageVisibility,
  waitToReconnect,
  watchSilence,
  type ReconnectWait,
  type SilenceWatch,
} from '@demicodes/web-ui/transport/liveness'
// The live view (`packages/plugin-mobile/src/index.ts`): its frames, the decoder that
// shows its pictures, and the viewer's input in the picture's coordinates,
// which every source's view shares.
export * from '@demicodes/web-ui/live/protocol'
export {
  LiveFrameAssembler,
  LiveFrameSplitter,
  LiveFrameReader,
  decodeAcks,
  encodeAcks,
  encodeControl,
  encodeVideo,
  framed,
  type LiveChunkAck,
  type LiveRawFrame,
  type LiveVideoChunk,
  type LiveVideoFrame,
} from '@demicodes/web-ui/live/frames'
export {
  CanvasPictures,
  picturesSupported,
  type LivePictureStream,
  type PictureHandlers,
  type PictureSink,
} from '@demicodes/web-ui/live/pictures'
export {
  ALT,
  CONTROL,
  ClickCount,
  META,
  SHIFT,
  keyInput,
  modifiers,
  pointerInput,
  touchInput,
  wheelInput,
  type KeyInput,
  type ModifierState,
  type PointerInput,
  type WheelInput,
} from '@demicodes/web-ui/live/input'
export {
  LiveLink,
  REFUSED_FRAME,
  SILENT_STREAM,
  type LiveConnection,
  type LiveFrames,
  type LiveLinkHandlers,
  type LiveLinkState,
} from '@demicodes/web-ui/live/link'
export {
  DRAWN_FRAME_ROOM,
  SQUARE_CORNERS,
  drawnFrame,
  observeSettledSize,
  scaledCorners,
  screenClip,
  screenPath,
  type CornerPoint,
  type FrameBox,
  type FrameButton,
  type DrawnFrameShape,
  type ScreenCorner,
  type ScreenCorners,
} from '@demicodes/web-ui/live/drawn-frame'
export { default as LiveDrawnFrame } from '@demicodes/web-ui/live/LiveDrawnFrame.vue'
export { default as LiveActivityOverlay } from '@demicodes/web-ui/live/LiveActivityOverlay.vue'
// Composables.
// A field that takes the focus as its surface opens, as a phone allows.
export { useAutofocus } from '@demicodes/web-ui/ui/autofocus'
export {
  vContextMenu,
  type ContextMenuDeclaration,
  type ContextMenuEntry,
  type ContextMenuGroup,
  type ContextMenuValue,
} from '@demicodes/web-ui/ui/context-menu'

// Files: the shapes the conversation files service gives, and their paths.
export {
  emptyChangeSet,
  type ChangeFile,
  type ChangeMode,
  type ChangeSetSource,
  type ChangeSources,
  type ReadCallChange,
} from '@demicodes/web-ui/files/changes'
// A request's changes: what the `edit` intent names, and the files the Change view lists.
export {
  requestEditSelectionSchema,
  sameEditRef,
  sameRequestFiles,
  type RequestChangeSource,
  type RequestEdit,
  type RequestEditRef,
  type RequestEditSelection,
  type RequestFile,
  type TranscriptRequest,
} from '@demicodes/web-ui/files/request-changes'
export type { FileBrowserSource } from '@demicodes/web-ui/files/types'
export { baseName, joinPath, relativePath } from '@demicodes/web-ui/files/paths'
export { downloadUrl } from '@demicodes/web-ui/files/download'
export { formatByteRatio, formatBytes } from '@demicodes/web-ui/files/format'
export { treeLayout } from '@demicodes/web-ui/files/tree-layout'

// Components: settings, controls, progress, navigation, files and icons.
export { default as AddressBar } from '@demicodes/web-ui/agent/AddressBar.vue'
export { default as Button } from '@demicodes/web-ui/ui/Button.vue'
export type { Action, Operation } from '@demicodes/web-ui/ui/operations'
export { default as ChangeView } from '@demicodes/web-ui/files/ChangeView.vue'
export { default as ConfirmDialog } from '@demicodes/web-ui/ui/ConfirmDialog.vue'
export { default as Dialog } from '@demicodes/web-ui/ui/Dialog.vue'
export { default as Dropdown } from '@demicodes/web-ui/ui/Dropdown.vue'
export { default as ExternalLink } from '@demicodes/web-ui/ui/ExternalLink.vue'
export { default as Fold } from '@demicodes/web-ui/ui/Fold.vue'
export { default as FileIcon } from '@demicodes/web-ui/files/FileIcon.vue'
export { default as FileView } from '@demicodes/web-ui/files/FileView.vue'
export { default as AttachmentView } from '@demicodes/web-ui/files/AttachmentView.vue'
export { default as LineCounts } from '@demicodes/web-ui/files/LineCounts.vue'
export { default as FoldChevron } from '@demicodes/web-ui/ui/FoldChevron.vue'
export { default as IconButton } from '@demicodes/web-ui/ui/IconButton.vue'
export { default as InlineError } from '@demicodes/web-ui/ui/InlineError.vue'
export { default as IndeterminateSpinner } from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
export { default as Menu } from '@demicodes/web-ui/ui/Menu.vue'
export { default as MenuDivider } from '@demicodes/web-ui/ui/MenuDivider.vue'
export { default as MenuGroup } from '@demicodes/web-ui/ui/MenuGroup.vue'
export { default as MenuItem } from '@demicodes/web-ui/ui/MenuItem.vue'
export { default as Popover } from '@demicodes/web-ui/ui/Popover.vue'
export { default as ProgressLine } from '@demicodes/web-ui/ui/ProgressLine.vue'
export { default as RegionStatus } from '@demicodes/web-ui/ui/RegionStatus.vue'
export { default as ScrollArea } from '@demicodes/web-ui/ui/ScrollArea.vue'
export { default as SettingsGroup } from '@demicodes/web-ui/settings/SettingsGroup.vue'
export { default as SettingsPage } from '@demicodes/web-ui/settings/SettingsPage.vue'
export { default as SettingsRow } from '@demicodes/web-ui/settings/SettingsRow.vue'
export { default as Switch } from '@demicodes/web-ui/ui/Switch.vue'
export { default as TextInput } from '@demicodes/web-ui/ui/TextInput.vue'
export { default as Toolbar } from '@demicodes/web-ui/ui/Toolbar.vue'
export type { ToolbarAction } from '@demicodes/web-ui/ui/Toolbar.vue'
export { default as Tooltip } from '@demicodes/web-ui/ui/Tooltip.vue'
export { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
// A system's logo, as a device's mark draws it.
export { default as SystemLogo } from '@demicodes/web-ui/devices/DeviceLogo.vue'
