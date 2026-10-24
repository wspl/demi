import { failureHeadline } from '../agent/error-detail'
import { formatPast, formatUntil } from '../ui/dates'
import { t } from './locales/text'
import {
  WEEKDAYS,
  type ConversationSchedule,
  type MonthDay,
  type Repeat,
  type RepeatUnit,
  type RunNote,
  type ScheduleTask,
  type Timing,
  type Trigger,
  type Weekday,
} from './types'

/** A clock time as the design writes it: 09:00. */
function clock(hour: number, minute: number): string {
  return `${String(hour).padStart(2, '0')}:${String(minute).padStart(2, '0')}`
}

/** The clock reading of `moment` in `zone`, the browser's own without one. */
function wallClock(moment: Date, zone?: string): { hour: number; minute: number } {
  const parts = new Intl.DateTimeFormat('en-US', {
    timeZone: zone,
    hourCycle: 'h23',
    hour: 'numeric',
    minute: 'numeric',
  }).formatToParts(moment)
  const part = (type: Intl.DateTimeFormatPartTypes) => Number(parts.find((each) => each.type === type)?.value)
  return { hour: part('hour'), minute: part('minute') }
}

/**
 * A moment to the minute, short: Oct 17 09:00, as the clocks of `zone` read
 * it, a schedule's times in the schedule's zone; the browser's zone without
 * one.
 */
export function shortMoment(iso: string, zone?: string): string {
  const moment = new Date(iso)
  const day = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', timeZone: zone }).format(moment)
  const { hour, minute } = wallClock(moment, zone)
  return `${day} ${clock(hour, minute)}`
}

/** A moment of the week, as Calendar's weekday list reads it: Thu 09:00, in `zone`. */
function weekMoment(iso: string, zone: string): string {
  const moment = new Date(iso)
  const day = new Intl.DateTimeFormat(undefined, { weekday: 'short', timeZone: zone }).format(moment)
  const { hour, minute } = wallClock(moment, zone)
  return `${day} ${clock(hour, minute)}`
}

/** An interval in minutes in its largest whole unit. */
function intervalOf(minutes: number): { count: number; unit: 'minutes' | 'hours' | 'days' } {
  if (minutes % 1440 === 0) {
    return { count: minutes / 1440, unit: 'days' }
  }
  if (minutes % 60 === 0) {
    return { count: minutes / 60, unit: 'hours' }
  }
  return { count: minutes, unit: 'minutes' }
}

/** An interval in words: Every 10 min, Every 2 hours. */
function intervalWords(minutes: number): string {
  const { count, unit } = intervalOf(minutes)
  return t(`words.interval.${unit}`, { count }, count)
}

/** A weekday's name in the page's language: Mon, or Monday for the long width. */
function weekdayName(day: Weekday, width: 'short' | 'long' = 'short'): string {
  // 2023-01-02 was a Monday.
  return new Intl.DateTimeFormat(undefined, { weekday: width }).format(new Date(2023, 0, 2 + WEEKDAYS.indexOf(day)))
}

/** Weekdays and a time in words: Every day 09:00, Weekdays 09:00, Mon 10:00, Mon, Thu 09:00. */
function weekdaysWords(time: string, days: readonly Weekday[]): string {
  const chosen = WEEKDAYS.filter((day) => days.includes(day))
  if (chosen.length === 7) {
    return t('words.daily', { time })
  }
  if (chosen.length === 5 && !chosen.includes('sat') && !chosen.includes('sun')) {
    return t('words.weekdays', { time })
  }
  return t('words.days', { days: chosen.map((day) => weekdayName(day)).join(', '), time })
}

/** A month's short name in the page's language: Mar. */
function monthName(month: number): string {
  return new Intl.DateTimeFormat(undefined, { month: 'short' }).format(new Date(2023, month - 1, 1))
}

/** A repeat in words, as a sentence starts: Every week, Every 3 months. */
function repeatWords(repeat: Repeat): string {
  return t(`words.repeat.${repeat.unit}`, { count: repeat.every }, repeat.every)
}

/** A day of a month in words: Day 15, Last day, Second Tuesday, Last working day. */
function monthDayWords(day: MonthDay): string {
  switch (day.kind) {
    case 'date':
      return t('words.date', { date: day.date })
    case 'last_day':
      return t('words.lastDay')
    case 'weekday':
      return t(`words.week.${day.week}`, { weekday: weekdayName(day.weekday, 'long') })
    case 'working_day':
      return t(`words.workingDay.${day.which}`)
  }
}

/** A task's moment in words, as the repeat's unit reads it: 09:00, Mon 10:00, Last working day 17:00, Mar 15 09:00. */
function taskWords(task: ScheduleTask, unit: RepeatUnit): string {
  switch (unit) {
    case 'days':
      return task.time
    case 'weeks':
      return weekdaysWords(task.time, task.days)
    case 'months':
      return t('words.moment', { day: monthDayWords(task.day), time: task.time })
    case 'years': {
      const day = task.day.kind === 'date'
        ? t('words.monthDate', { month: monthName(task.month), date: task.day.date })
        : t('words.dayOfMonth', { day: monthDayWords(task.day), month: monthName(task.month) })
      return t('words.moment', { day, time: task.time })
    }
  }
}

/**
 * A cron expression in words where it is one of the common shapes, at a
 * time every day, on weekdays or on one weekday; any other reads as itself.
 */
function cronWords(expression: string): string {
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

function timingWords(timing: Timing, zone: string, now: Date | number): string {
  switch (timing.kind) {
    case 'once': {
      // Within a week the weekday names the day; further out, the date does.
      const near = new Date(timing.at).getTime() - new Date(now).getTime() < 6 * 86_400_000
      return t('trigger.once', { when: near ? weekMoment(timing.at, zone) : shortMoment(timing.at, zone) })
    }
    case 'repeat': {
      const only = timing.tasks.length === 1 ? timing.tasks[0]! : null
      const what = only ? taskWords(only, timing.repeat.unit) : t('words.tasks', { count: timing.tasks.length }, timing.tasks.length)
      return `${repeatWords(timing.repeat)} · ${what}`
    }
    case 'every':
      return intervalWords(timing.minutes)
    case 'cron':
      return cronWords(timing.expression)
  }
}

/** The trigger in words: Every week · Mon 10:00, When output changes · 5 min. */
export function triggerWords(schedule: Pick<ConversationSchedule, 'trigger' | 'timeZone'>, now: Date | number): string {
  const trigger: Trigger = schedule.trigger
  switch (trigger.kind) {
    case 'time':
      return timingWords(trigger.timing, schedule.timeZone, now)
    case 'change': {
      const { count, unit } = intervalOf(trigger.everyMinutes)
      return t('trigger.change', { interval: t(`trigger.interval.${unit}`, { count }, count) })
    }
    case 'url':
      return t('trigger.url')
  }
}

/** Why a run was skipped or failed, in words: MacBook Pro was offline. */
export function noteWords(note: RunNote): string {
  switch (note.kind) {
    case 'offline':
      return t('note.offline', { device: note.device })
    case 'limit':
      return t('note.limit', { limit: note.limit, calls: note.calls }, note.calls)
    case 'command_timed_out':
      return t('note.timedOut')
    case 'command_failed':
      return t('note.commandFailed', { message: note.message })
    // A turn's failure reads as the transcript words it; a run that could not start its turn says why.
    case 'turn_failed':
      return note.code ? failureHeadline(note.code, null, new Date()) : note.message
  }
}

/**
 * When the schedule wakes the conversation, as a time from now; for a change
 * watch or a URL, when it last did. Null while it is paused, which says so.
 */
export function wakeWords(schedule: ConversationSchedule, now: Date | number): string | null {
  if (schedule.paused) {
    return null
  }
  if (schedule.trigger.kind === 'time') {
    return schedule.nextAt ? formatUntil(schedule.nextAt, now) : null
  }
  return schedule.lastRun ? formatPast(schedule.lastRun.at, now) : t('notYet')
}

/**
 * Why the last wake failed or was skipped, as its note says, or that it did
 * when it left none; null when it did neither.
 */
export function lastWakeReason(schedule: ConversationSchedule): string | null {
  const run = schedule.lastRun
  if (!run || (run.state !== 'failed' && run.state !== 'skipped')) {
    return null
  }
  return run.note ? noteWords(run.note) : t(run.state === 'failed' ? 'wake.failed' : 'wake.skipped')
}
