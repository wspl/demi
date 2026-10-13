import type { ComposerAttachmentInput } from '@demicodes/web-ui/agent/message-input/attachments'

/** A draft a specimen starts with: its message, its files and how long ago it was saved. */
export interface SeedDraft {
  text: string
  files?: ComposerAttachmentInput[]
  minutesAgo: number
}

/** Two drafts, the older with a file. */
export const twoDrafts: SeedDraft[] = [
  { text: 'Check prod’s DNS next, and tell me whether the AAAA record still points at the old host.', minutesAgo: 12 },
  {
    text: 'Compare the two screenshots and list what moved.',
    files: [{ id: 'before', name: 'before.png', phase: 'ready' }, { id: 'after', name: 'after.png', phase: 'ready' }],
    minutesAgo: 190,
  },
]

/** Seven drafts, more than the list shows at once, so it scrolls. */
export const sevenDrafts: SeedDraft[] = [
  ...twoDrafts,
  { text: 'Rename the session cookie in the docs as well as in the code.', minutesAgo: 60 * 26 },
  { text: 'Write a release note for the Drafts work.\nKeep it to three lines.', minutesAgo: 60 * 30 },
  {
    text: 'Summarize this log for the on-call handoff.',
    files: [{ id: 'ci-log', name: 'ci.log', phase: 'ready' }],
    minutesAgo: 60 * 24 * 3,
  },
  { text: 'Ask whether the Windows runner still needs the long-path setting.', minutesAgo: 60 * 24 * 5 },
  { text: 'Draft an email to the vendor about the usage limits endpoint returning 500 after midnight UTC.', minutesAgo: 60 * 24 * 9 },
]
