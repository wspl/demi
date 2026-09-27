import { expect, test } from 'bun:test'
import { draftPreview } from '../message-input/draft-preview'
import { ATTACHMENT_MARK } from '../../markdown/user-markdown'

test('a replaced draft reads as its first line that holds anything, each capsule as its file\'s name', () => {
  const cases: [string, string[], string][] = [
    [`\n   \nCompare ${ATTACHMENT_MARK} with ${ATTACHMENT_MARK}.\nKeep the fix small.`, ['before.png', 'after.png'], 'Compare before.png with after.png.'],
    [`${ATTACHMENT_MARK}\nRead this first`, ['trace.txt'], 'trace.txt'],
    [`Fix the login${ATTACHMENT_MARK}bug`, ['trace.txt'], 'Fix the login trace.txt bug'],
    ['  Fix the **login** bug  ', [], 'Fix the **login** bug'],
  ]
  for (const [markdown, names, preview] of cases) {
    expect(draftPreview(markdown, names)).toBe(preview)
  }
})
