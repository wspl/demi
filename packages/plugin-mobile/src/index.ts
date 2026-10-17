// The `mobile` plugin's page (`plugin-pages.md`, `mobile.md` § The live
// view): its `mobile` work panel kind, a device's live view, its panel
// session, and the types of its state, parameters, results and stream,
// generated from the plugin's manifest.
import { definePage } from '@demicodes/plugin-sdk'
import { PLUGIN } from './generated/plugin'
import { mobileDeviceKind } from './live/kind'
import { MobileDevicesController, type MobileControllerOptions } from './live/controller'
import { mobileApi } from './devices'

export * from './generated/plugin'
export { mobileApi, MobileError, type MobileApi } from './devices'
export { MobileDevicesController, type MobileControllerOptions } from './live/controller'
export { mobileDeviceKind, mobileTabDataSchema, type MobileTabData } from './live/kind'

/** The page, whose panel sessions take `options`: the pictures' support, the document's own unless the gallery gives it. */
export function mobilePage(options: MobileControllerOptions = {}) {
  return definePage({
    plugin: PLUGIN,
    kinds: [mobileDeviceKind],
    panel: (conversation, page) =>
      new MobileDevicesController(
        mobileApi(page.plugin.conversation(conversation)),
        page.errors,
        options,
      ),
  })
}

/** The page the registry imports (`plugin-pages.md` § Registration). */
export default mobilePage()
