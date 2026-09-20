---
'@demicodes/web-ui': patch
---

`TextInput` takes `trim`: whitespace around the value is dropped as it arrives, in the field as in the model. The subscription token and the API key fields use it, so a copy that brought spaces or a newline from a terminal still signs in.
