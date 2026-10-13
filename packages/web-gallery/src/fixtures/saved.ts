import type { ComposerAttachmentInput } from '@demicodes/web-ui/agent/message-input/attachments'

/** A message saved for later that a specimen starts with: its text and its files, and whether the server did not save it. */
export interface SeedSaved {
  text: string
  files?: ComposerAttachmentInput[]
  notSaved?: boolean
}

/** Two saved messages, the older with files. */
export const twoSaved: SeedSaved[] = [
  { text: 'Check the server’s DNS next, and tell me whether the AAAA record still points at the old host.' },
  {
    text: 'Compare the two screenshots and list what moved.',
    files: [{ id: 'before', name: 'before.png', phase: 'ready' }, { id: 'after', name: 'after.png', phase: 'ready' }],
  },
]

/** Two saved messages under one the server did not save, as when the conversation kept 200 already. */
export const notSavedOnTop: SeedSaved[] = [
  { text: 'Then check whether the CDN still caches the old certificate.', notSaved: true },
  ...twoSaved,
]

/** Seven saved messages, more than the list shows at once, so it scrolls. */
export const sevenSaved: SeedSaved[] = [
  ...twoSaved,
  { text: 'Rename the session cookie in the docs as well as in the code.' },
  { text: 'Write a release note for Saved for Later.\nKeep it to three lines.' },
  {
    text: 'Summarize this log for the on-call handoff.',
    files: [{ id: 'ci-log', name: 'ci.log', phase: 'ready' }],
  },
  { text: 'Ask whether the Windows runner still needs the long-path setting.' },
  { text: 'Draft an email to the vendor about the usage limits endpoint returning 500 after midnight UTC.' },
]
