package invalid

// +demi:pattern ^[a[b]]$
// +demi:root direction=receive
type Broken string
