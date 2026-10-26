/**
 * Settings › Updates's state, as the backend says it
 * (`crates/web-api-protocol/src/server_upgrade.rs`), and what the sidebar
 * reads of it.
 */
import type { StepState } from '../ui/update-steps'

/** Which releases Settings › Updates offers: the published ones, or every one. */
export type UpgradeChannel = 'beta' | 'canary'

/** What the last look for a newer release found. */
export type UpgradeCheck =
  | { state: 'never' }
  | { state: 'checking' }
  | { state: 'checked', at: string, newer?: string | null, notesUrl?: string | null }
  | { state: 'failed', at: string, reason: string }

/** The steps an update takes while the server serves, in order. */
export type UpgradeStepKind =
  | 'download'
  | 'unpack'
  | 'checkConfiguration'
  | 'downloadGvisor'
  | 'installImage'
  | 'checkDatabases'
  | 'restart'

export interface UpgradeStep {
  kind: UpgradeStepKind
  state: StepState
}

/**
 * How far the server is in getting a newer release ready ahead of Update
 * Now: downloaded and unpacked, its gVisor and Cloud image installed, and its
 * runner on the devices.
 */
export type UpgradePreparation =
  | { state: 'none' }
  | { state: 'downloading', version: string, done: number, total: number }
  | { state: 'installing', version: string }
  | { state: 'ready', version: string }
  | { state: 'failed', version: string, reason: string }

/** An update that runs, or did not happen. */
export type UpgradeRun =
  | { state: 'none' }
  | { state: 'updating', to: string, steps: UpgradeStep[] }
  | { state: 'failed', to: string, reason: string, steps: UpgradeStep[] }

/** Settings › Updates: the release the server runs, its channel, the check and the run. */
export interface ServerUpgrade {
  running: string
  channel: UpgradeChannel
  check: UpgradeCheck
  preparation: UpgradePreparation
  run: UpgradeRun
}

/** Whether `state` offers a release newer than the running one, which nothing updates to yet. */
export function updateOffered(state: ServerUpgrade): string | null {
  if (state.run.state === 'updating') {
    return null
  }
  return state.check.state === 'checked' ? (state.check.newer ?? null) : null
}
