/**
 * Settings › Updates's state, as the backend says it
 * (`crates/web-api-protocol/src/server_upgrade.rs`), and what the sidebar
 * reads of it.
 */
/** Which releases Settings › Updates offers: the published ones, or every one. */
export type UpgradeChannel = 'beta' | 'canary'

/** What the last look for a newer release found. */
export type UpgradeCheck =
  | { state: 'never' }
  | { state: 'checking' }
  | { state: 'checked', at: string, newer?: string | null, notesUrl?: string | null }
  | { state: 'failed', at: string, reason: string }

/** An update that runs, or did not happen. */
export type UpgradeRun =
  | { state: 'none' }
  | { state: 'downloading', to: string, bytesDone: number, bytes: number, bytesPerSecond: number }
  | { state: 'preparing', to: string }
  | { state: 'restarting', to: string }
  | { state: 'failed', to: string, reason: string }

/** Settings › Updates: the release the server runs, its channel, the check and the run. */
export interface ServerUpgrade {
  running: string
  channel: UpgradeChannel
  check: UpgradeCheck
  run: UpgradeRun
}

/** Whether `state` offers a release newer than the running one, which nothing updates to yet. */
export function updateOffered(state: ServerUpgrade): string | null {
  if (state.run.state !== 'none' && state.run.state !== 'failed') {
    return null
  }
  return state.check.state === 'checked' ? (state.check.newer ?? null) : null
}
