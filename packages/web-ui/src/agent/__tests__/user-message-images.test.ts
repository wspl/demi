import { expect, test } from 'bun:test'
import { imageCapsule } from '../message-editor/capsules'

// A user's message shows the images it names as attachment capsules, as a file
// the user attached, never as a picture in its text (`file-previews.md`
// § Files named in messages). `ImageNodeView` renders the capsule this
// describes, in the sent bubble and in the composer alike; the Markdown stays
// as typed. No I/O; a few milliseconds.

test('a web image the user typed is a capsule named as the user alt-texted it, its picture the web address', () => {
  expect(imageCapsule({ url: 'https://example.com/shots/a%20b.png?v=2' }, 'https://example.com/shots/a%20b.png?v=2', 'A checkerboard'))
    .toEqual({ id: 'image:https://example.com/shots/a%20b.png?v=2', name: 'A checkerboard', image: 'https://example.com/shots/a%20b.png?v=2' })
})

test('a Host image is a capsule at its path, its picture the version of the file the page keeps', () => {
  expect(imageCapsule({ path: '/tmp/x/shot.png' }, '/raw?path=/tmp/x/shot.png&version=v2', 'A checkerboard'))
    .toEqual({ id: 'image:/tmp/x/shot.png', name: 'A checkerboard', image: '/raw?path=/tmp/x/shot.png&version=v2', path: '/tmp/x/shot.png' })
})

test('an image with no alt text is named by its file, else by its kind', () => {
  expect(imageCapsule({ path: '/tmp/x/shot.png' }, '/raw', '').name).toBe('shot.png')
  expect(imageCapsule({ url: 'https://example.com/shots/a%20b.png?v=2' }, 'https://example.com/shots/a%20b.png?v=2', '').name).toBe('a b.png')
  expect(imageCapsule({ url: 'data:image/png;base64,AAAA' }, 'data:image/png;base64,AAAA', '').name).toBe('image')
})
