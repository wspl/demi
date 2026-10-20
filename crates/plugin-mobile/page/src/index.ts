// The `mobile` plugin's page (`packages/plugin-sdk/src/index.ts`): its `mobile`
// work panel kind (a device's live view), its store of each conversation's
// simulators, and the types of
// its state, parameters, results and stream, generated from the plugin's
// manifest.
//
// The view is the browser's live view with a different source. Its pictures and
// its control are shared (`crates/live-view`, `@demicodes/plugin-sdk`), and
// only what a source alone has stays here, the simulator's frame and buttons.
// The toolbar's controls each run the operation of the command that does the
// same and wait for nothing, the picture being the answer; a rotation turns
// the shown picture at once (`docs/decisions/0007-rotation-shows-at-once.md`).
import { definePage } from '@demicodes/plugin-sdk'
import { PLUGIN } from './generated/plugin'
import { mobileDeviceKind } from './live/kind'
import { mobileStore } from './live/store'
import type { MobileControllerOptions } from './live/controller'

export * from './generated/plugin'
export { mobileApi, MobileError, type MobileApi } from './devices'
export { MobileDevicesController, type MobileControllerOptions } from './live/controller'
export { mobileDeviceKind, mobileTabDataSchema, type MobileTabData } from './live/kind'

/** The page, whose simulators take `options`: the pictures' support, the document's own unless the gallery gives it. */
export function mobilePage(options: MobileControllerOptions = {}) {
  return definePage({
    plugin: PLUGIN,
    kinds: [mobileDeviceKind(mobileStore(options))],
  })
}

/** The page the registry imports (`crates/xtask/src/contracts/registry.rs`). */
export default mobilePage()
