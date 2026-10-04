# Browser manifest fixture

`manifest.json` is the browser plugin's expected manifest, including its object
member order and numeric spelling. The test compacts whitespace only and
compares the factory's encoded manifest byte for byte; it does not regenerate
expectations from Go.
