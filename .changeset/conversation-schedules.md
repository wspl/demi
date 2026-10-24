---
"@demicodes/web-ui": minor
"@demicodes/web": minor
"@demicodes/web-gallery": minor
"@demicodes/protocol": minor
---

A conversation has schedules, which the agent sets with `demi schedule` and which wake that conversation only. A chip above the composer, *3 Scheduled*, opens them as a menu (a sheet on a phone), where a switch turns a schedule off and on and Delete removes it, on hover or by swiping left. Automations, their page, editor and sidebar entry go; `ConversationMetadata` carries the schedules and `ConversationSummary` says whether a conversation is scheduled.

On upgrade:

- An automation that woke an existing conversation becomes that conversation's schedule.
- An automation that started a new conversation each run is deleted with its run history; the conversations its runs started now show in the sidebar.
- A URL trigger no longer checks Stripe's signatures: a call signed by Stripe alone is answered 401 until the caller sends the trigger's secret.
