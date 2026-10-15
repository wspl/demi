import { afterEach, expect, test } from 'bun:test'
import { createPageI18n } from '../../i18n/text'
import type { Language } from '../../i18n/languages'
import { formatAge, formatDay, formatModified, formatMoment, formatPast, formatReset, formatSpan, formatUntil } from '../dates'

// The page speaks English unless a test says otherwise.
afterEach(() => {
  createPageI18n('en')
})

// When a usage window resets, as Settings and the context card both say it
// (`usage-and-quota.md` § What a snapshot says), against a fixed clock in the
// local time zone: Sunday, October 11, 2026, 12:22 PM. Each row of the table,
// and each side of every boundary between two rows.
const now = new Date(2026, 9, 11, 12, 22)

const SECOND = 1_000
const MINUTE = 60 * SECOND
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR

/** The moment `ms` from now. */
function inMs(ms: number): string {
  return new Date(now.getTime() + ms).toISOString()
}

/** A moment on the local clock. */
function at(...parts: [number, number, number, number, number, number?, number?]): string {
  return new Date(...parts).toISOString()
}

test.each([
  { scenario: 'the clock has passed', at: inMs(-30 * SECOND), wording: 'Resets now' },
  { scenario: 'the clock reaches', at: inMs(0), wording: 'Resets now' },
  { scenario: 'within a minute', at: inMs(SECOND), wording: 'Resets in <1m' },
  { scenario: '59 s ahead', at: inMs(59 * SECOND), wording: 'Resets in <1m' },
  { scenario: '60 s ahead', at: inMs(MINUTE), wording: 'Resets in 1m' },
  { scenario: 'within the hour, counted down', at: inMs(42 * MINUTE + 59 * SECOND), wording: 'Resets in 42m' },
  { scenario: '59 m 59 s ahead', at: inMs(59 * MINUTE + 59 * SECOND), wording: 'Resets in 59m' },
  { scenario: 'an hour ahead', at: inMs(HOUR), wording: 'Resets in 1h' },
  { scenario: 'within 24 hours', at: inMs(3 * HOUR + 42 * MINUTE), wording: 'Resets in 3h 42m' },
  { scenario: 'on the hour', at: inMs(3 * HOUR), wording: 'Resets in 3h' },
  { scenario: '23 h 59 m ahead', at: inMs(23 * HOUR + 59 * MINUTE), wording: 'Resets in 23h 59m' },
  { scenario: '24 h ahead', at: inMs(DAY), wording: 'Resets Mon 12:22 PM' },
  { scenario: 'within 7 days', at: at(2026, 9, 17, 16, 0), wording: 'Resets Sat 4:00 PM' },
  { scenario: 'a moment just before the minute', at: at(2026, 9, 17, 15, 59, 59, 600), wording: 'Resets Sat 4:00 PM' },
  { scenario: '6 d 23 h ahead', at: inMs(6 * DAY + 23 * HOUR), wording: 'Resets Sun 11:22 AM' },
  { scenario: '7 d ahead', at: inMs(7 * DAY), wording: 'Resets Oct 18' },
  { scenario: 'later this year', at: at(2026, 9, 24, 16, 0), wording: 'Resets Oct 24' },
  { scenario: 'on the last day of the year', at: at(2026, 11, 31, 23, 0), wording: 'Resets Dec 31' },
  { scenario: 'in another year', at: at(2027, 0, 3, 9, 5), wording: 'Resets Jan 3, 2027' },
  { scenario: 'a moment rounding into another year', at: at(2026, 11, 31, 23, 59, 59, 600), wording: 'Resets Jan 1, 2027' },
])('a reset $scenario reads "$wording"', ({ at, wording }) => {
  expect(formatReset(at, now)).toBe(wording)
})

// Every wording names a time on the 12-hour clock, in the local time zone.
test.each([
  { scenario: 'an afternoon', at: '2026-09-02T18:12:00', wording: 'Sep 2, 2026 6:12 PM' },
  { scenario: 'a morning', at: '2025-03-02T09:05:00', wording: 'Mar 2, 2025 9:05 AM' },
  { scenario: 'midnight', at: '2026-09-07T00:00:00', wording: 'Sep 7, 2026 12:00 AM' },
  { scenario: 'noon', at: '2026-09-07T12:30:00', wording: 'Sep 7, 2026 12:30 PM' },
  { scenario: 'an unknown time', at: undefined, wording: '' },
  { scenario: 'a time that is not one', at: 'never', wording: '' },
])('a modified time of $scenario reads "$wording"', ({ at, wording }) => {
  expect(formatModified(at)).toBe(wording)
})

test.each([
  { scenario: 'this year', at: at(2026, 9, 9, 14, 0), wording: 'Friday, October 9 at 2:00 PM' },
  { scenario: 'another year', at: at(2025, 0, 3, 9, 5), wording: 'Friday, January 3, 2025 at 9:05 AM' },
])('a moment of $scenario reads "$wording"', ({ at, wording }) => {
  expect(formatMoment(at, now)).toBe(wording)
})

test('a day reads in full, without a time', () => {
  expect(formatDay(at(2026, 9, 7, 14, 0))).toBe('October 7, 2026')
})

// A message's time against the page's clock, which ticks each second while
// the backend stamps the message: a reply that just arrived is in the past.
test.each([
  { scenario: 'a moment the clock has not reached yet', at: inMs(201), wording: 'a few seconds ago' },
  { scenario: 'five minutes ago', at: inMs(-5 * MINUTE), wording: '5 minutes ago' },
])('a past time of $scenario reads "$wording"', ({ at, wording }) => {
  expect(formatPast(at, now)).toBe(wording)
})

test.each([
  { scenario: 'eight minutes ahead', at: inMs(8 * MINUTE), wording: 'in 8 minutes' },
  { scenario: 'a moment the clock has passed', at: inMs(-SECOND), wording: 'in a few seconds' },
])('a time to come of $scenario reads "$wording"', ({ at, wording }) => {
  expect(formatUntil(at, now)).toBe(wording)
})

// The duration words of the thinking label, the command time tag, the agent
// status bar and the reset times: each unit floored, the seconds never shown
// from the hour on.
test.each([
  { scenario: 'a moment', ms: 0, wording: '0s' },
  { scenario: 'under a minute', ms: 42_900, wording: '42s' },
  { scenario: 'on the minute', ms: 180_000, wording: '3m' },
  { scenario: 'minutes and seconds', ms: 134_900, wording: '2m 14s' },
  { scenario: 'an hour and seconds', ms: 3_659_000, wording: '1h' },
  { scenario: 'hours and minutes', ms: 3_930_000, wording: '1h 5m' },
  { scenario: 'more than a day', ms: 97_200_000, wording: '27h' },
  { scenario: 'a clock set back', ms: -5_000, wording: '0s' },
])('a span of $scenario reads "$wording"', ({ ms, wording }) => {
  expect(formatSpan(ms)).toBe(wording)
})

// Each language says a date and a time as Intl says them in it, and the
// compact forms Intl has no words for from its catalogue
// (`localization.md` § Where the text lives).
test.each<{ language: Language; age: string; day: string; past: string; reset: string; span: string }>([
  { language: 'en', age: '2h', day: 'October 7, 2026', past: '5 minutes ago', reset: 'Resets in 3h 42m', span: '2m 14s' },
  { language: 'zh-Hans', age: '2 小时', day: '2026年10月7日', past: '5分钟前', reset: '3 小时 42 分钟后重置', span: '2 分 14 秒' },
  { language: 'zh-Hant', age: '2 小時', day: '2026年10月7日', past: '5分鐘前', reset: '3 小時 42 分鐘後重設', span: '2 分 14 秒' },
  { language: 'ja', age: '2 時間', day: '2026年10月7日', past: '5 分前', reset: '3 時間 42 分後にリセット', span: '2 分 14 秒' },
  { language: 'ko', age: '2시간', day: '2026년 10월 7일', past: '5분 전', reset: '3시간 42분 후 초기화', span: '2분 14초' },
])('in $language an age reads "$age" and a day "$day"', ({ language, age, day, past, reset, span }) => {
  createPageI18n(language)
  expect(formatAge(inMs(-2 * HOUR - MINUTE), now)).toBe(age)
  expect(formatDay(at(2026, 9, 7, 14, 0))).toBe(day)
  expect(formatPast(inMs(-5 * MINUTE), now)).toBe(past)
  expect(formatReset(inMs(3 * HOUR + 42 * MINUTE), now)).toBe(reset)
  expect(formatSpan(134_900)).toBe(span)
})
