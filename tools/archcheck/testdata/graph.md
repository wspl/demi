# Architecture fixture

### Go packages

The graph allows imports rather than requiring every edge to be used.

```text
internal/independent -> none
internal/core -> none
internal/framewire -> internal/core
```

### Other packages

```text
not-a-go-package -> none
```
