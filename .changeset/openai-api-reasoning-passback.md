---
'@demicodes/provider-openai-api': minor
---

Add opt-in `request.passBackReasoningContent` for Chat Completions: replay `assistant_thinking` as `reasoning_content`, and always include the field on tool-call assistant messages (empty string when a round had no thinking). Required for DeepSeek-style thinking + tool loops; leave off for official OpenAI.
