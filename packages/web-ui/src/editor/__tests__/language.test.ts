import { afterEach, expect, spyOn, test } from 'bun:test'
import { LanguageDescription } from '@codemirror/language'
import { waitFor } from '@demicodes/utils'
import { reportError, reportPageErrorsWith, type PageErrorReport } from '../../infra/errors'
import { loadFileLanguage } from '../language'

// Only a defect is reported (`backend.md` § The page's own errors): a
// language whose chunk did not download is the environment's failure, and
// one that threw as it was built is a defect. The language data's own load
// stands in for the chunk; no I/O, a few milliseconds.

let stop: (() => void) | null = null

afterEach(() => {
  stop?.()
  stop = null
})

test.each([
  { failure: 'a chunk that did not download', error: new TypeError('Failed to fetch dynamically imported module: /assets/python.js'), reported: false },
  { failure: 'a language that threw', error: new TypeError('m.python is not a function'), reported: true },
])('a file whose language fails, as $failure, shows plain; reported: $reported', async ({ error, reported }) => {
  const reports: PageErrorReport[] = []
  stop = reportPageErrorsWith(async (report) => {
    reports.push(report)
    return null
  })
  const load = spyOn(LanguageDescription.prototype, 'load').mockRejectedValue(error)
  try {
    expect(await loadFileLanguage('main.py')).toEqual([])
    // Reports go in order: once this defect is in, the language's would be too.
    reportError('Could Not Open the File', new Error('handler threw'))
    await waitFor(() => reports.some((report) => report.title === 'Could Not Open the File'))
    expect(reports.map((report) => report.message)).toEqual([...reported ? [error.message] : [], 'handler threw'])
  } finally {
    load.mockRestore()
  }
})
