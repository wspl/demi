import type { ComposerAttachmentInput } from '@demicodes/web-ui/agent/message-input/attachments'

/** A draft a specimen starts with: its message and its files. */
export interface SeedDraft {
  text: string
  files?: ComposerAttachmentInput[]
}

/** Two drafts, the older with a file. */
export const twoDrafts: SeedDraft[] = [
  { text: 'Check prod’s DNS next, and tell me whether the AAAA record still points at the old host.' },
  {
    text: 'Compare the two screenshots and list what moved.',
    files: [{ id: 'before', name: 'before.png', phase: 'ready' }, { id: 'after', name: 'after.png', phase: 'ready' }],
  },
]

/** Seven drafts, more than the list shows at once, so it scrolls. */
export const sevenDrafts: SeedDraft[] = [
  ...twoDrafts,
  { text: 'Rename the session cookie in the docs as well as in the code.' },
  { text: 'Write a release note for the Drafts work.\nKeep it to three lines.' },
  {
    text: 'Summarize this log for the on-call handoff.',
    files: [{ id: 'ci-log', name: 'ci.log', phase: 'ready' }],
  },
  { text: 'Ask whether the Windows runner still needs the long-path setting.' },
  { text: 'Draft an email to the vendor about the usage limits endpoint returning 500 after midnight UTC.' },
]
