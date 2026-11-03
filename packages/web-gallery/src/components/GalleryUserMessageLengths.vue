<script setup lang="ts">
import { productWould } from '../product-would'
import type { UserContentBlock } from '@demicodes/protocol'
import UserBlock from '@demicodes/web-ui/agent/blocks/UserBlock.vue'
import { shownComment, type ShownComment } from '@demicodes/web-ui/agent/comments/comments'
import { provideMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import type { ConversationFiles } from '@demicodes/web-ui/markdown/types'
import { demoImageUrl, longUserText } from '../fixtures/blocks'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'
import type { SentenceText } from '@demicodes/web-ui/ui/ui-text'

/**
 * Every length a user message comes in, each in a frame whose right edge
 * drags to another width, so the cut can be watched following the wrapping.
 */
const props = defineProps<{
  /** The gallery project: the images the messages name, and the panel a click opens them in. */
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

/** A comment on a reply's picture, its words under the picture. */
const pictureComment: ShownComment = shownComment({
  id: 'lengths-picture-comment',
  blockId: 'lengths-reply',
  start: 0,
  end: 1,
  at: 0,
  quote: 'chart.png',
  picture: { type: 'url', url: demoImageUrl },
  content: text('Why does the line drop on Tuesday? Compare it with the deploy times in the release log and say which change it follows.'),
})

const specimens: {
  variant: SentenceText
  content: UserContentBlock[]
  comments?: ShownComment[]
  pending?: 'steer' | 'queued'
  actions?: boolean
}[] = [
  { variant: 'One line', content: text('Rename the session cookie in the login test.') },
  { variant: 'Five lines · whole', content: text(steps.join('\n')) },
  {
    variant: 'Under the fold · whole',
    content: text([...steps, '', ...steps, '', 'Expected the sign-in page; got a blank screen.'].join('\n')),
  },
  {
    variant: 'Over the fold · folds, with Show More',
    content: text([longUserText, ...steps, longUserText].join('\n\n')),
  },
  {
    variant: 'One paragraph · the cut follows the width',
    content: text(longUserText.split('\n\n').slice(0, 2).join(' ')),
  },
  { variant: 'Paragraphs', content: text(longUserText) },
  { variant: 'A code block', content: text(code) },
  { variant: 'A table, as typed', content: text(table) },
  {
    variant: 'A path with no spaces',
    content: text('The failing import: /Users/alex/Projects/shop/packages/web-ui/src/agent/blocks/UserBlock.vue/../../markdown/message-files/../render/../md/../types/../../../../web/src/conversation/changes.ts'),
  },
  {
    variant: 'Chinese',
    content: text('登录测试在我们把会话 cookie 从 sid 改名为 session 之后开始失败。主分支和这个分支上的 CI 都是红的。辅助函数写出的 Set-Cookie 仍然正确，出问题的是断言：它还在找 sid= 前缀，以及一个我们已经不再发送的 Session 头。请只改 auth.test.ts，不要重命名辅助函数，也不要动 cookie.ts。过期 cookie 的用例可以留到下一次。'),
  },
  {
    variant: 'A file in the text',
    content: [
      { type: 'text', text: 'The capture ' },
      { type: 'image', source: { type: 'url', url: demoImageUrl } },
      {
        type: 'attachment',
        name: 'login-fail.png',
        id: 'a1',
        mediaType: 'image/png',
        sizeBytes: 48211,
        sha256: 'demo-png',
      },
      { type: 'text', text: ` shows it. ${longUserText}` },
    ],
  },
  { variant: 'Queued · two lines, the second fading', content: text(longUserText), pending: 'queued' },
  {
    variant: 'Pending steer · two lines, the second fading',
    content: text(steps.join('\n')),
    pending: 'steer',
  },
  { variant: 'Queued · one line, whole', content: text('Also add a case for the expired cookie.'), pending: 'queued' },
  {
    variant: 'Queued · one line, then a paragraph: the line alone',
    content: text('Schedule “Nightly build”, at 02:00\n\nRun the nightly build and tell me what failed.'),
    pending: 'queued',
  },
  {
    variant: 'Queued · a comment on a picture, its words folding',
    content: [],
    comments: [pictureComment],
    pending: 'queued',
  },
  { variant: 'Actions on the last line', content: text(longUserText), actions: true },
]
</script>

<template>
  <GallerySection
    title="User Message Length"
    note="A message taller than 300px folds to about 200px, as Claude Code's does, its last line fading out, and Show More under it opens it whole; Show Less folds it again. A message waiting to be sent, queued or as a steer, folds once it runs past two lines, to its first line whole and its second fading, or to its first line alone when a paragraph ends there, and keeps Show More first in its row of controls, then Send and Recall. A message under 300px shows whole, since folding it would hide less than the control takes. A line across the fold shows whole, code's as text's. A file is a capsule on its line and counts as text. Drag a frame’s right edge: the fold follows the wrapping."
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
              :comments="specimen.comments"
              :pending="specimen.pending"
              :actions-pinned="specimen.actions"
              @send-now="productWould('Send the Waiting Message Now')"
              @recall="productWould('Recall the Waiting Message')"
            />
          </div>
        </div>
      </GallerySpecimen>
    </div>
  </GallerySection>
</template>
