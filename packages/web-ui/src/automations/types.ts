import type { ModelSettings } from '../agent/model-selection'

/**
 * An automation as the page shows and edits it (`automations.md`): what
 * starts it, where it runs, what it sends, and its runs, newest first. Its
 * fields are the web API's, with the target's place named as the host menu
 * names it.
 */
export interface Automation {
  id: string
  name: string
  trigger: AutomationTrigger
  /** The time zone its times and its daily limit are read in, such as Asia/Shanghai. */
  timeZone: string
  /** The most runs a day, for a change watch or a URL; none without a limit. */
  maxRunsPerDay: number | null
  target: AutomationTarget
  prompt: string
  /** The model settings a new conversation starts with; none for the user's own. */
  model: ModelSettings | null
  /** A paused one runs only with Run Now. */
  paused: boolean
  /** Why it paused itself; none when the user paused it or it is on. */
  pausedBecause: PauseReason | null
  /** A URL's address, `https://<server>/hooks/<id>`; none for another trigger. */
  address: string | null
  /** A URL trigger checks Stripe's signatures with the signing secret the user pasted. */
  stripeSigning: boolean
  /** When it next starts by itself, ISO 8601; none while paused, with no time left, or for a URL. */
  nextAt: string | null
  runs: AutomationRun[]
}

export type Weekday = 'mon' | 'tue' | 'wed' | 'thu' | 'fri' | 'sat' | 'sun'

/** Monday first, as the days a daily schedule names are listed. */
export const WEEKDAYS: readonly Weekday[] = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun']

/** When a time trigger fires: once, daily on chosen weekdays, every interval, or a cron expression of five fields. */
export type TimeSchedule =
  | { kind: 'once'; at: string }
  | { kind: 'daily'; time: string; days: Weekday[] }
  | { kind: 'every'; minutes: number }
  | { kind: 'cron'; expression: string }

/** What starts an automation's runs. */
export type AutomationTrigger =
  | { kind: 'time'; schedule: TimeSchedule }
  | { kind: 'change'; command: string; everyMinutes: number }
  | { kind: 'url' }

/**
 * Where an automation's runs work: a new conversation each run, on a Host
 * and in a project on it or none; or an existing conversation, which has
 * its own place.
 */
export type AutomationTarget =
  | { kind: 'new'; hostId: string; projectId: string | null }
  | { kind: 'existing'; conversationId: string }

export type PauseReason = 'conversation_archived' | 'conversation_deleted'

/** Why a run was skipped or failed. */
export type RunNote =
  | { kind: 'offline'; device: string }
  | { kind: 'limit'; limit: number; calls: number }
  | { kind: 'target_gone' }
  | { kind: 'no_model' }
  | { kind: 'command_timed_out' }
  | { kind: 'command_failed'; message: string }
  | { kind: 'turn_failed'; message: string; code: string | null }

/** One run: what the trigger started, and how it ended. */
export interface AutomationRun {
  id: string
  /** When it started, ISO 8601. */
  at: string
  state: RunState
  note: RunNote | null
  /** The start of the agent's last answer; none before it answered. */
  line: string | null
  /** The run's conversation; none for a run that started none, such as a skipped one. */
  conversationId: string | null
  /** A run that notified, whose conversation the user has not opened since. */
  unread: boolean
  /** Its new conversation is still filed under the automation, out of the sidebar. */
  filed: boolean
}

export type RunState = 'running' | 'done' | 'notified' | 'skipped' | 'failed'

/** A Host a target can name, as the host menu lists them. */
export interface AutomationHost {
  /** `cloud`, or the device's id. */
  id: string
  name: string
  kind: 'cloud' | 'device'
  state: 'online' | 'offline' | 'updating'
  /** Its projects. */
  projects: readonly { id: string; name: string }[]
}

/** A conversation an automation can wake, with the Host it runs on. */
export interface AutomationConversation {
  id: string
  title: string
  hostId: string
}

/** An automation as a conversation it wakes lists it: its chip's menu. */
export interface AutomationLink {
  id: string
  name: string
  /** Its trigger in words: Weekdays 09:00. */
  trigger: string
  paused: boolean
}

/** A change of an automation's parts, as the editor makes it; the Stripe secret is written, never read. */
export type AutomationChange = Partial<Pick<Automation, 'name' | 'trigger' | 'timeZone' | 'maxRunsPerDay' | 'target' | 'prompt' | 'model' | 'paused'>> & {
  /** Stripe's signing secret the user pasted; null to stop checking Stripe's signatures. */
  stripeSecret?: string | null
}
