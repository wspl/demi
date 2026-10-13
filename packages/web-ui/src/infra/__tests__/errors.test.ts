import { afterEach, expect, test } from 'bun:test'
import { formError, reportError, reportPageErrorsWith, shownError, type PageErrorReport } from '../errors'
import { dismissToast, toasts } from '../toast'

// An error the page shows names the request it came from, and a failure the
// page found itself is reported to the backend, whose ID the toast then names
// (`backend.md` § Request IDs and the server log). A reporter stand-in in
// place of the backend; no I/O, a few milliseconds.

let stopReports: (() => void) | null = null

afterEach(() => {
  stopReports?.()
  stopReports = null
  for (const toast of [...toasts]) dismissToast(toast.id)
})

/** A request the backend refused, as the app's API errors carry it. */
class Refused extends Error {
  constructor(message: string, readonly requestId: string | null) {
    super(message)
  }
}

/** Records each report and answers with `answer`. */
function reporter(answer: string | null) {
  const reports: PageErrorReport[] = []
  stopReports = reportPageErrorsWith(async (report) => {
    reports.push(report)
    return answer
  })
  return reports
}

/** Lets the report's answer land. */
async function settle(): Promise<void> {
  for (let tick = 0; tick < 5; tick++)
    await Promise.resolve()
}

test('a failed request is shown with its ID and is not reported again', async () => {
  const reports = reporter('req_unused000000000')
  reportError('Could Not Save the Provider', new Refused('The endpoint is unavailable.', 'req_k3v9x2m7qd4h8w1p'), { userVisible: true })
  await settle()
  expect(toasts.map(({ title, message, requestId }) => ({ title, message, requestId }))).toEqual([
    { title: 'Could Not Save the Provider', message: 'The endpoint is unavailable.', requestId: 'req_k3v9x2m7qd4h8w1p' },
  ])
  expect(reports).toEqual([])
  expect(shownError(new Refused('Busy', 'req_a'))).toEqual({ message: 'Busy', requestId: 'req_a' })
})

test('the page’s own failure is reported with where it failed, and its toast names the report once it is in', async () => {
  const reports = reporter('req_pagereport00001')
  reportError('Could Not Read the Plugin’s Data', new Error('Invalid server response'), {
    userVisible: true,
    detail: '✖ Invalid input: expected string, received number\n  → at sections[0].title',
  })
  expect(toasts[0]!.requestId).toBeNull()
  await settle()
  expect(reports).toEqual([{
    title: 'Could Not Read the Plugin’s Data',
    message: 'Invalid server response',
    detail: '✖ Invalid input: expected string, received number\n  → at sections[0].title',
  }])
  expect(toasts[0]!.requestId).toBe('req_pagereport00001')
})

test('a report the backend refused leaves the toast without an ID, and an expected failure is not reported', async () => {
  const reports = reporter(null)
  reportError('Could Not Open the File', new Error('handler threw'), { userVisible: true })
  reportError('Could Not Copy', new Error('denied'), { expected: true })
  await settle()
  expect(reports.map((report) => report.title)).toEqual(['Could Not Open the File'])
  // Without a detail of its own, the report says where by the error's stack.
  expect(reports[0]!.detail).toContain('errors.test.ts')
  expect(toasts.map((toast) => toast.requestId)).toEqual([null])
})

test('a report keeps an address’s path and drops its query', async () => {
  const reports = reporter('req_report0000000002')
  reportError('Could Not Open the File', new Error('GET https://demi.example/api/fs/raw?path=/home/ana/.env&token=abc failed; /api/fs/file?path=secret.txt too'))
  await settle()
  expect(reports[0]!.message).toBe('GET https://demi.example/api/fs/raw failed; /api/fs/file too')
})

test('a refusal the user corrects in the form names no request and says the page’s words; any other failure names its request', () => {
  const corrections = { invalid_credentials: 'That is not your current password.' }
  class Coded extends Refused {
    constructor(readonly code: string, message: string) {
      super(message, 'req_k3v9x2m7qd4h8w1p')
    }
  }
  expect(formError(new Coded('invalid_credentials', 'Current password is wrong'), corrections))
    .toEqual({ message: 'That is not your current password.', requestId: null })
  expect(formError(new Coded('internal_error', 'The store failed'), corrections))
    .toEqual({ message: 'The store failed', requestId: 'req_k3v9x2m7qd4h8w1p' })
})
