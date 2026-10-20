# Localization

Demi is available in English, Simplified Chinese, Traditional Chinese,
Japanese and Korean, each complete. A user who picks 简体中文 in Settings › General sees
every control, message and email in Chinese at once, without a reload. What a
model reads and what people wrote stay as they are.

This page says what is translated and how a word is chosen. The gallery's
Writing page says how each language is written, its glossary included.

The code answers how the language is chosen, where each text lives and how
text from the backend is worded. These files hold the answers:

- `packages/web-ui/src/i18n/`
- `packages/web-ui/src/infra/errors.ts`
- `crates/web-api-protocol/src/error.rs`

## How a word is chosen

English comes first, and the other languages follow it. Each language says
what the English says, in the words its users would find most natural for the
thing in front of them. It is never a literal rendering, and no word chosen for
another language changes the English.

No single source decides a text. Choose each term and each phrasing by
comparing many products that speak the language well and say the same thing.
Compare these kinds of products:

- AI apps of the same kind (Codex, ChatGPT, Claude, Cursor)
- The system's own apps (macOS and iOS: Mail, Finder, System Settings)
- For developer concepts, such as commit, branch, terminal or diff, the tools
  developers read every day (VS Code, Git clients)

Weigh the products by these rules:

- Where they agree, take theirs.
- Where they differ, take the one a user of that region reads without stopping.
- Prefer everyday words over a vendor's house terminology, such as 归档 over
  存档 and 账户 over 帐户.
- A word a product uses for a different thing that shares an English name does
  not count. A model vendor is 供应商, not an editor's 提供程序.

The glossary records each choice with the products it was weighed against. A
better choice found later replaces it everywhere.

## What is translated

Demi translates every text that it shows a person itself, in these places:

- The web app
- Every plugin's pages
- The gallery's own controls and its specimens
- The notifications the page shows
- Emails
- The pages the backend renders itself, such as the sign-in callback

Demi never translates the following:

- What a model reads: the agent's guide and system prompt, tool results,
  `demi` command help and output. It stays English in every language, so
  the agent's behavior does not depend on the user's.
- What people and models wrote: messages, replies, summaries and the titles
  a model gives conversations, which follow the language the user writes in
  (`crates/agent-server/src/title.rs`).
- Names: files, paths, commands, models, vendors, branches, accounts, and
  the product's own. See [The product's name](#the-products-name).
- The gallery's notes, which document a section for the people building
  Demi, and the content of its fixtures, such as a specimen conversation's
  messages, which stand for what users write.
- A vendor's or a system's own words quoted as they came, such as a
  provider's response, git's output or an operating system's error. They
  show under their disclosure, below a headline Demi words in the user's
  language from the failure's code.

### The product's name

In a sentence, the name is *Demi*, as in *Demi needs permission* and *Add Demi
to Your Home Screen*. The license's names, the *Demi License* and *The Demi
Authors*, are written the same way.

Where the name stands alone as the product's mark, it is the wordmark *demi*,
in lowercase in every language. Amazon's and Intel's logos work the same way,
while their sentences say Amazon and Intel. The wordmark appears in these
places:

- The sidebar's header
- The end of the page's title, as in *整理季度报告 · demi*
- The installed app's name under its icon and in the Dock
- The name beside the logo on the sign-in page
- A notification's title when it has none of its own
- The name a passkey shows for the site
- The name of the certificate that signs the Mac runner, *demi Code Signing*
