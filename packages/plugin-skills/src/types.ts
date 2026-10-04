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
}

/** A SKILL.md of a source that is not a skill, and why. */
export interface SettingsSkippedSkill {
  path: string
  reason: string
}

/** A git repository the user added, pinned to one commit. */
export interface SettingsSkillSource {
  id: string
  origin: string
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
}
