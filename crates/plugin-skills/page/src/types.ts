import type { DeviceMark, DeviceState } from '@demicodes/plugin-sdk'

/** One skill of a source, as the Skills page shows it. */
export interface SettingsSkill {
  name: string
  description: string
  /** What its SKILL.md breaks of the format; it loads all the same. */
  warnings: string[]
  /** Off keeps the files but hides them from the agent. */
  enabled: boolean
  /** The skill is never offered to the agent, even on. */
  disableModelInvocation: boolean
  /** While off, the origin of the source whose skill that is on has its name. */
  takenBy?: string
  /** The place of that source: Global or its workspace, by name. */
  takenIn?: string
}

/** A SKILL.md of a source that is not a skill, and why. */
export interface SettingsSkippedSkill {
  path: string
  reason: string
}

/** A paired device that can fetch a source, drawn as every device is: by its mark and state. */
export interface SettingsSkillDevice {
  id: string
  name: string
  state: DeviceState
  mark: DeviceMark | null
  downloadPercent: number | null
}

/** What fetches a source: the server, or a paired device; a device no longer paired is `removed`. */
export type SettingsSkillFetcher =
  | { kind: 'server' }
  | { kind: 'device'; id: string; name: string; online: boolean }
  | { kind: 'removed'; id: string }

/** A git repository the user added, pinned to one commit. */
export interface SettingsSkillSource {
  id: string
  origin: string
  fetcher: SettingsSkillFetcher
  /** The pinned commit; none until a fetch succeeded. */
  commit?: string
  fetching: boolean
  /** The last fetch's failure, shown until a fetch succeeds. */
  failure?: { at: string; message: string }
  /** The repository's default branch points to another commit than the pinned one. */
  updateAvailable: boolean
  skills: SettingsSkill[]
  skipped: SettingsSkippedSkill[]
}

export interface SettingsSkillDraft {
  origin: string
  /** The device that fetches it; null for the server. */
  fetcher: string | null
}

/**
 * How adding a source ended, as one of:
 *
 * - Added, which closes the dialog.
 * - Refused for its input, which the dialog says under the field, keeping what
 *   was typed.
 * - Failed for another reason, which the host has already reported.
 */
export type AddSourceAnswer =
  | { kind: 'added' }
  | { kind: 'refused'; message: string }
  | { kind: 'failed' }
