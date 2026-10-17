import { failureHeadline } from '../agent/error-detail'
import { formatPast } from '../ui/dates'
import { t } from './locales/text'
import {
  WEEKDAYS,
  type Automation,
  type AutomationConversation,
  type AutomationHost,
  type AutomationRun,
  type PauseReason,
  type RunNote,
  type RunState,
  type TimeSchedule,
  type Weekday,
} from './types'

/** A clock time as the design writes it: 09:00. */
function clock(hour: number, minute: number): string {
  return `${String(hour).padStart(2, '0')}:${String(minute).padStart(2, '0')}`
}

/** The clock reading of `moment` in `zone`, the browser's own without one. */
function wallClock(moment: Date, zone?: string): { year: number; month: number; day: number; hour: number; minute: number } {
  const parts = new Intl.DateTimeFormat('en-US', {
    timeZone: zone,
    hourCycle: 'h23',
    year: 'numeric',
    month: 'numeric',
    day: 'numeric',
    hour: 'numeric',
    minute: 'numeric',
  }).formatToParts(moment)
  const part = (type: Intl.DateTimeFormatPartTypes) => Number(parts.find((each) => each.type === type)?.value)
  return { year: part('year'), month: part('month'), day: part('day'), hour: part('hour'), minute: part('minute') }
}

/**
 * A moment to the minute, short: Oct 17 09:00, as the clocks of `zone`
 * read it, an automation's times in the automation's zone
 * (`automations.md` § Triggers); the browser's zone without one.
 */
export function shortMoment(iso: string, zone?: string): string {
  const moment = new Date(iso)
  const day = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', timeZone: zone }).format(moment)
  const { hour, minute } = wallClock(moment, zone)
  return `${day} ${clock(hour, minute)}`
}

/** A moment as the Once field shows it in `zone`: 2026-10-18 08:00. */
export function fieldMoment(iso: string, zone: string): string {
  const { year, month, day, hour, minute } = wallClock(new Date(iso), zone)
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${year}-${pad(month)}-${pad(day)} ${clock(hour, minute)}`
}

/**
 * The moment `zone`'s clocks show the date and time at; a time the clocks
 * skip is the first minute after the gap, and a repeated one its first
 * occurrence, as the scheduler reads them.
 */
export function zonedMoment(year: number, month: number, day: number, hour: number, minute: number, zone: string): Date {
  const wall = Date.UTC(year, month - 1, day, hour, minute)
  /** How far `zone`'s clocks are ahead of UTC at `at`. */
  const offset = (at: number) => {
    const read = wallClock(new Date(at), zone)
    return Date.UTC(read.year, read.month - 1, read.day, read.hour, read.minute) - Math.floor(at / 60_000) * 60_000
  }
  // The offsets a day before and after hold every reading of the time.
  const candidates = [offset(wall - 86_400_000), offset(wall + 86_400_000)]
    .map((each) => wall - each)
    .filter((at) => offset(at) === wall - at)
    .sort((a, b) => a - b)
  if (candidates.length > 0) {
    return new Date(candidates[0]!)
  }
  // In a gap: read with the later offset, the time falls before the
  // change, and with the earlier one after it; the change, found to the
  // minute between them, is the first minute after the gap.
  const before = offset(wall - 86_400_000)
  let low = wall - offset(wall + 86_400_000)
  let high = wall - before
  while (high - low > 60_000) {
    const middle = low + Math.floor((high - low) / 120_000) * 60_000
    if (offset(middle) === before) {
      low = middle
    } else {
      high = middle
    }
  }
  return new Date(high)
}

/** An interval in minutes as the editor shows it: the largest whole unit. */
export function intervalOf(minutes: number): { count: number; unit: 'minutes' | 'hours' | 'days' } {
  if (minutes % 1440 === 0) {
    return { count: minutes / 1440, unit: 'days' }
  }
  if (minutes % 60 === 0) {
    return { count: minutes / 60, unit: 'hours' }
  }
  return { count: minutes, unit: 'minutes' }
}

/** The minutes of a count of a unit. */
export function minutesOf(count: number, unit: 'minutes' | 'hours' | 'days'): number {
  return count * { minutes: 1, hours: 60, days: 1440 }[unit]
}

/** An interval in words: Every 10 min, Every 2 hours. */
export function intervalWords(minutes: number): string {
  const { count, unit } = intervalOf(minutes)
  return t(`words.interval.${unit}`, { count }, count)
}

/** A weekday's short name in the page's language: Mon. */
export function weekdayName(day: Weekday, width: 'short' | 'long' = 'short'): string {
  // 2023-01-02 was a Monday.
  return new Intl.DateTimeFormat(undefined, { weekday: width }).format(new Date(2023, 0, 2 + WEEKDAYS.indexOf(day)))
}

/** A daily schedule in words: Every day 09:00, Weekdays 09:00, Mon, Thu 09:00. */
export function dailyWords(time: string, days: readonly Weekday[]): string {
  const chosen = WEEKDAYS.filter((day) => days.includes(day))
  if (chosen.length === 7) {
    return t('words.daily', { time })
  }
  if (chosen.length === 5 && !chosen.includes('sat') && !chosen.includes('sun')) {
    return t('words.weekdays', { time })
  }
  if (chosen.length === 1) {
    return t('words.weekly', { day: weekdayName(chosen[0]!, 'long'), time })
  }
  return t('words.days', { days: chosen.map((day) => weekdayName(day)).join(', '), time })
}

/**
 * A cron expression in words where it is one of the common shapes, at a
 * time every day, on weekdays or on one weekday; any other reads as itself.
 */
export function cronWords(expression: string): string {
  const fields = expression.trim().split(/\s+/)
  if (fields.length !== 5) {
    return expression
  }
  const [minute, hour, dayOfMonth, month, weekday] = fields as [string, string, string, string, string]
  if (!/^\d+$/.test(minute) || !/^\d+$/.test(hour) || dayOfMonth !== '*' || month !== '*') {
    return expression
  }
  const time = clock(Number(hour), Number(minute))
  if (weekday === '*') {
    return t('words.daily', { time })
  }
  if (weekday === '1-5') {
    return t('words.weekdays', { time })
  }
  if (/^[0-6]$/.test(weekday)) {
    // Cron's weekday 0 is Sunday, the last of WEEKDAYS.
    return t('words.weekly', { day: weekdayName(WEEKDAYS[(Number(weekday) + 6) % 7]!, 'long'), time })
  }
  return expression
}

/** A time schedule in words, its moment in its zone `zone`. */
export function scheduleWords(schedule: TimeSchedule, zone?: string): string {
  switch (schedule.kind) {
    case 'once':
      return shortMoment(schedule.at, zone)
    case 'daily':
      return dailyWords(schedule.time, schedule.days)
    case 'every':
      return intervalWords(schedule.minutes)
    case 'cron':
      return cronWords(schedule.expression)
  }
}

/** The trigger in words, as the list shows it. */
export function triggerWords(automation: Pick<Automation, 'trigger' | 'timeZone'>): string {
  const trigger = automation.trigger
  switch (trigger.kind) {
    case 'time':
      return scheduleWords(trigger.schedule, automation.timeZone)
    case 'change':
      return t('words.onChange', { interval: intervalWords(trigger.everyMinutes) })
    case 'url':
      return t('words.url')
  }
}

/**
 * The target in words, as the list shows it: a new conversation with its
 * Host and project, or the existing conversation's title alone, which
 * names the place already.
 */
export function targetWords(
  automation: Automation,
  hosts: readonly AutomationHost[],
  conversations: readonly AutomationConversation[],
): string {
  const target = automation.target
  if (target.kind === 'existing') {
    const title = conversations.find((candidate) => candidate.id === target.conversationId)?.title
    return title ? t('words.existing', { title }) : t('words.existingGone')
  }
  const host = hosts.find((candidate) => candidate.id === target.hostId)
  const project = host?.projects.find((candidate) => candidate.id === target.projectId)
  return [t('words.newConversation'), host?.name ?? '', project?.name ?? ''].filter(Boolean).join(' · ')
}

/** A run's state in words. */
export function stateWords(state: RunState): string {
  return t(`runs.status.${state}`)
}

/** Why a run was skipped or failed, in words: MacBook Pro was offline. */
export function noteWords(note: RunNote): string {
  switch (note.kind) {
    case 'offline':
      return t('runs.note.offline', { device: note.device })
    case 'limit':
      return t('runs.note.limit', { limit: note.limit, calls: note.calls }, note.calls)
    case 'target_gone':
      return t('runs.note.targetGone')
    case 'no_model':
      return t('runs.note.noModel')
    case 'command_timed_out':
      return t('runs.note.timedOut')
    case 'command_failed':
      return t('runs.note.commandFailed', { message: note.message })
    // A turn's failure reads as the transcript words it; a run that could not start its turn says why.
    case 'turn_failed':
      return note.code ? failureHeadline(note.code, null, new Date()) : note.message
  }
}

/** A run's line: the start of the agent's last answer, or why it was skipped or failed. */
export function runLine(run: AutomationRun): string {
  return run.note ? noteWords(run.note) : run.line ?? ''
}

/** Why an automation paused itself, in words. */
export function pausedWords(reason: PauseReason | null): string {
  return reason ? t(`editor.pausedBecause.${reason}`) : t('editor.pausedHint')
}

/** The last run in words against `now`: Notified 2 hours ago; Never run. */
export function lastRunWords(run: AutomationRun | undefined, now: Date | number): string {
  if (!run) {
    return t('runs.never')
  }
  return t('runs.last', { status: stateWords(run.state), when: formatPast(run.at, now) })
}
