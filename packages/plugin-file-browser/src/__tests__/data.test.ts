import { expect, test } from 'bun:test'
import { firstFileData, goBack, goForward, showFile } from '../data'

// Cost: pure, under a millisecond.

test('each file opened replaces the one shown and keeps it for Back', () => {
  const second = showFile(showFile(firstFileData(), 'src/a.ts'), 'src/b.ts')
  expect(second).toEqual({ path: 'src/b.ts', back: ['src/a.ts'], forward: [] })
  const back = goBack(second)
  expect(back).toEqual({ path: 'src/a.ts', back: [], forward: ['src/b.ts'] })
  expect(goForward(back)).toMatchObject({ path: 'src/b.ts' })
  const replaced = showFile(back, 'src/c.ts')
  expect(replaced).toMatchObject({ path: 'src/c.ts', forward: [] })
  // Opening the file shown changes nothing.
  expect(showFile(replaced, 'src/c.ts')).toBe(replaced)
})
