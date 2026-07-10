---
'@demicodes/web-ui': patch
---

Make the shipped source clean under strict consumer tsconfigs: replace
constructor parameter properties in ConversationRuntime with explicit field
assignments (rejected by erasableSyntaxOnly) and drop the never-read
messageInputRef from ConversationView (rejected by noUnusedLocals). web-ui
publishes as source, so its code must compile under consumers' settings.
