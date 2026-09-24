<script setup lang="ts">
import type { UserContentBlock } from '@demicodes/protocol'
import UserBlock from '@demicodes/web-ui/agent/blocks/UserBlock.vue'
import { provideMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import type { ConversationFiles } from '@demicodes/web-ui/markdown/types'
import { demoImageUrl, longUserText } from '../fixtures/blocks'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * Every length a user message comes in, each in a frame whose right edge
 * drags to another width, so the cut can be watched following the wrapping.
 */
const props = defineProps<{
  /** The gallery workspace: the images the messages name, and the panel a click opens them in. */
  files: ConversationFiles
  cwd: string
}>()

provideMessageFiles(() => ({ ...props.files, cwd: props.cwd }))

function text(value: string): UserContentBlock[] {
  return [{ type: 'text', text: value }]
}

const steps = [
  'Steps to reproduce:',
  '1. Sign in with the demo account',
  '2. Open Settings, then Sessions',
  '3. Revoke the current session',
  '4. Reload the page',
]

const code = [
  'Why does this throw on the second call?',
  '```ts',
  'export function readSession(header: string | undefined) {',
  '  const pairs = header?.split(\';\') ?? []',
  '  for (const pair of pairs) {',
  '    const [name, value] = pair.trim().split(\'=\')',
  '    if (name === \'session\')',
  '      return value',
  '  }',
  '  throw new Error(\'no session cookie\')',
  '}',
  '```',
].join('\n')

const table = [
  'The runs from this morning:',
  '',
  '| Run | Branch | Result |',
  '| --- | --- | --- |',
  '| 4812 | main | failed |',
  '| 4813 | cookie-rename | failed |',
  '| 4815 | cookie-rename | passed |',
  '| 4816 | main | passed |',
].join('\n')

const specimens: { variant: string; content: UserContentBlock[]; pending?: boolean; actions?: boolean }[] = [
  { variant: 'one line', content: text('Rename the session cookie in the login test.') },
  { variant: 'five lines · whole', content: text(steps.join('\n')) },
  {
    variant: 'five lines with a blank one · whole',
    content: text(['The login test fails after the cookie rename.', '', ...steps.slice(0, 3)].join('\n')),
  },
  {
    variant: 'six lines · the fifth fades',
    content: text([...steps, 'Expected the sign-in page; got a blank screen.'].join('\n')),
  },
  {
    variant: 'one paragraph · the cut follows the width',
    content: text(longUserText.split('\n\n').slice(0, 2).join(' ')),
  },
  { variant: 'paragraphs', content: text(longUserText) },
  {
    variant: 'text, then an image',
    content: text('Reply with exactly this Markdown and nothing else:\n![The test pattern](assets/photo.png)\nSee [the README](README.md).'),
  },
  {
    variant: 'an image first',
    content: text('![The test pattern](assets/photo.png)\nThis is what the capture should show; ours is shifted left by a column.'),
  },
  { variant: 'a code block', content: text(code) },
  { variant: 'a table, as typed', content: text(table) },
  {
    variant: 'a path with no spaces',
    content: text('The failing import: /Users/zan/Projects/demi/packages/web-ui/src/agent/blocks/UserBlock.vue/../../markdown/message-files/../render/../md/../types/../../../../web/src/conversation/changes.ts'),
  },
  {
    variant: 'Chinese',
    content: text('登录测试在我们把会话 cookie 从 sid 改名为 session 之后开始失败。主分支和这个分支上的 CI 都是红的。辅助函数写出的 Set-Cookie 仍然正确，出问题的是断言：它还在找 sid= 前缀，以及一个我们已经不再发送的 Session 头。请只改 auth.test.ts，不要重命名辅助函数，也不要动 cookie.ts。过期 cookie 的用例可以留到下一次。'),
  },
  {
    variant: 'a file in the text',
    content: [
      { type: 'text', text: 'The capture ' },
      { type: 'image', source: { type: 'url', url: demoImageUrl } },
      {
        type: 'attachment',
        name: 'login-fail.png',
        path: '/home/demi/.demi/attachments/demo/login-fail.png',
        mediaType: 'image/png',
        sizeBytes: 48211,
        sha256: 'demo-png',
      },
      { type: 'text', text: ` shows it. ${longUserText}` },
    ],
  },
  { variant: 'pending', content: text(longUserText), pending: true },
  { variant: 'actions on the last line', content: text(longUserText), actions: true },
]
</script>

<template>
  <GallerySection
    title="User message length"
    note="A message longer than five lines shows its first five, and the fifth fades out. A line of text across the cut shows whole; an image or a code block across it is cut there. A file is a capsule on its line and counts as text. Drag a frame's right edge: the cut follows the wrapping."
  >
    <div class="specimen-stack specimen-stack-loose">
      <GallerySpecimen
        v-for="specimen in specimens"
        :key="specimen.variant"
        :variant="specimen.variant"
        wide
      >
        <!-- The wrapper resizes; room under the frame keeps its grip clear of the frame's corner. -->
        <div class="max-w-full resize-x overflow-hidden pb-3" style="width: 40rem; min-width: 16rem">
          <div
            class="gallery-frame gallery-user-frame bg-surface"
            :class="specimen.actions ? 'gallery-user-frame-actions' : ''"
          >
            <UserBlock
              :content="specimen.content"
              :pending="specimen.pending"
              :actions-pinned="specimen.actions"
            />
          </div>
        </div>
      </GallerySpecimen>
    </div>
  </GallerySection>
</template>
