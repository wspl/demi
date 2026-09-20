---
'@demicodes/backend': patch
---

A Cloud reset holds only the conversations whose target is that Cloud; a conversation that merely has it attached keeps running. What the user sends to a held conversation now waits for the transition instead of being refused with `conversation_busy`, so the conversation works again by itself once the transition ends.
