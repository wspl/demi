import type { Block, ModelSelection } from '@demicodes/core'
import type { Conversation } from './types'

/** Curated transcript fixtures use the same block contract as live conversation rendering. */
export function populateTranscript(c: Conversation, model: ModelSelection, index: number): void {
  let sequence = 0
  const meta = () => {
    const order = sequence++
    return {
      id: `${c.id}-sample-${order}`,
      createdAt: new Date(Date.parse(c.updatedAt) - 180_000 + order * 8_000).toISOString(),
      model,
    }
  }
  const user = (text: string): Block => ({
    ...meta(),
    type: 'user',
    turnId: `${c.id}-turn-${sequence}`,
    preamble: null,
    content: [{ type: 'text', text }],
  })
  const answer = (text: string): Block => ({ ...meta(), type: 'text', text })
  const thinking = (text: string): Block => ({ ...meta(), type: 'thinking', text, signature: null })
  const usage = (): Block => ({
    ...meta(),
    type: 'response',
    usage: {
      inputTokens: 1820 + index * 73,
      outputTokens: 640 + index * 17,
      cacheReadTokens: 1024,
      cacheWriteTokens: 0,
    },
  })
  const command = (script: string, description: string, stdout: string, failed = false): Block => ({
    ...meta(),
    type: 'tool_call',
    toolUseId: `${c.id}-tool-${sequence}`,
    toolName: 'shell_exec',
    input: JSON.stringify({ script, description }),
    status: failed ? 'error' : 'completed',
    streamingOutput: [],
    output: [{ type: 'text', text: stdout }],
    view: { chunks: [{ stream: failed ? 'stderr' : 'stdout', text: stdout }] },
  })

  c.status = 'done'
  if (c.id === 'reading') {
    c.blocks = [
      user(
        'I have a few quiet hours this weekend. Help me pick something to read, with a mix of fiction and essays.',
      ),
      answer(
        'Try a short reading list instead of a stack you feel obliged to finish.\n\n| Book | Mood | A good starting point |\n| --- | --- | --- |\n| A Wizard of Earthsea | Reflective fantasy | The first two chapters |\n| The Little Prince | Brief and contemplative | Read it in one sitting |\n| A Room of One’s Own | Thoughtful essay | The opening walk |\n\nChoose **one** for Saturday and keep the others as alternatives.',
      ),
      user('Let’s go with Earthsea. Can you suggest a relaxed schedule without spoilers?'),
      answer(
        '### Saturday\n\n- [ ] Read the opening two chapters with coffee.\n- [ ] Put the book down for a walk.\n- [ ] Write down one image or sentence you want to remember.\n\n### Sunday\n\nContinue only if you feel curious. There is no chapter target.\n\n> A good weekend reading plan leaves room for the book to surprise you.',
      ),
      usage(),
    ]
  } else if (c.id === 'web-review' || c.id === 'sidebar-motion') {
    c.blocks = [
      user(
        'Review the sidebar ordering implementation. Make sure activity does not override a manual order.',
      ),
      thinking(
        'The rendering order and stored order need to agree. I will inspect grouping, then check the reorder operation with a focused example.',
      ),
      command(
        'rg -n "sortConversations|updatedAt" packages/web-ui/src/sidebar',
        'Inspect sidebar grouping',
        'group-conversations.ts:9:export function sortConversations(\ntypes.ts:18:  updatedAt: string\n',
      ),
      answer(
        'Keep the incoming order stable within each pin partition:\n\n```ts\nconst ordered = [...conversations].sort((a, b) =>\n  Number(b.pinned) - Number(a.pinned),\n)\n```\n\nThe comparator returns zero for peers, so a later response cannot move a manually placed row.',
      ),
      command(
        'bun test packages/web-ui/src/sidebar',
        'Verify sidebar behavior',
        '12 pass\n0 fail\n36 expect() calls\n',
      ),
      answer(
        'The ordering cases pass. I also checked that empty projects retain their position and that moving a conversation does not reorder projects.',
      ),
      usage(),
    ]
  } else if (c.id === 'demi-3') {
    c.blocks = [
      user('Run the interface checks and summarize any failures.'),
      command(
        'bun run typecheck:web',
        'Check browser types',
        'packages/web/src/settings/SettingsDialog.vue: Property "label" is missing.\n',
        true,
      ),
      answer(
        'The check stopped on a missing label in the settings form. I found the failing field and was about to inspect its props.',
      ),
      {
        ...meta(),
        type: 'error',
        code: 'connection_error',
        message: 'The connection was interrupted before the next response completed.',
      },
    ]
    c.status = 'error'
  } else if (c.id === 'workspace-flow') {
    c.blocks = [
      user('Compare the recent-directory menu with the full directory browser.'),
      thinking(
        'The menu should optimize returning to a known workspace; the browser should support exploring a different location.',
      ),
      answer(
        'The two paths serve different tasks:\n\n1. **Recent directories** — select a known path in one step.\n2. **Choose another directory** — browse folders, inspect a file, then confirm the working directory.\n\nI will check how both paths behave when the selected device goes offline.',
      ),
      { ...meta(), type: 'abort', isResumed: false },
    ]
    c.status = 'aborted'
  } else if (c.id === 'release-plan') {
    c.blocks = [
      user('Make a release checklist for the web interface.'),
      answer(
        '### Interface checks\n\n- [x] Verify sidebar ordering.\n- [x] Confirm menus close on outside click.\n- [ ] Check reduced-motion behavior.\n- [ ] Review narrow-window layouts.\n\n### Release checks\n\n- [ ] Run scoped tests and type checks.\n- [ ] Build the frontend bundle.\n- [ ] Review the final changeset.',
      ),
      user('Keep the accessibility and keyboard checks in the summary too.'),
      {
        ...meta(),
        type: 'compaction_boundary',
        summary:
          'Release scope: sidebar ordering, outside-click dismissal, reduced motion, narrow layouts, keyboard navigation and accessible control names. Run scoped tests, type checks and a production build before review.',
        summaryTokens: 61,
      },
      answer(
        'Added keyboard navigation and accessible control names to the review criteria. The next step is checking the sidebar with the mouse set aside.',
      ),
      usage(),
    ]
  } else if (c.projectId === 'notes') {
    c.blocks = [
      user(`Help me work on “${c.title}”. I want the writing to be clear and concrete.`),
      answer(
        `## ${c.title}\n\nStart with a scene the reader can picture:\n\n> The room was quiet except for the keyboard. On the screen, three unfinished ideas waited for a decision.\n\nThen explain the choice in one sentence. Avoid summarizing every possibility before the reader knows what matters.\n\n| Draft | Revision |\n| --- | --- |\n| There are a number of considerations | Three things matter |\n| It is possible to observe | You can see |\n| In order to begin | To begin |`,
      ),
      user('Can you make the opening shorter and warmer?'),
      answer(
        '“Three ideas were waiting on the screen. I picked the one I wanted to return to tomorrow.”\n\nThis keeps the concrete image and gives the paragraph a clear direction.',
      ),
      usage(),
    ]
  } else if (c.projectId === 'demi') {
    c.blocks = [
      user(`Take a look at ${c.title.toLowerCase()} and give me a concrete checklist.`),
      thinking(
        'I will separate visible behavior from state changes and focus on a few reproducible interactions.',
      ),
      answer(
        `### ${c.title}\n\n1. Start with an existing conversation.\n2. Perform the action once and check the visible result.\n3. Repeat it with a long title and a narrow sidebar.\n4. Verify keyboard access and reduced motion.\n\n\`state → action → result\` should remain easy to follow.\n\n\`\`\`ts\nconst before = items.map(item => item.id)\nperformAction()\nexpect(items.map(item => item.id)).toEqual(before)\n\`\`\``,
      ),
      usage(),
    ]
  } else {
    c.blocks = [
      user(`Let’s spend a little time on ${c.title.toLowerCase()}.`),
      answer(
        `## ${c.title}\n\nStart with one small decision you can make today. Keep the plan short enough that you can see the next step without rereading it.\n\n- **Today:** collect two or three options.\n- **Tomorrow:** choose the one that feels most useful.\n- **Later:** revisit the alternatives if your priorities change.`,
      ),
      user('I only have about half an hour. What should I do first?'),
      answer(
        'Use the first ten minutes to write down what you want from this. Spend fifteen minutes on one promising option, then leave five minutes to decide what comes next.\n\nYou can stop there; a clear next step is enough.',
      ),
      usage(),
    ]
  }
  c.unread = c.status === 'error' || c.status === 'aborted' || index % 7 === 0
}
