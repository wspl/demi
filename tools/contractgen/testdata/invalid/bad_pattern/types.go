package invalid

// +demi:pattern ^\d+$
// +demi:root direction=receive
type Broken string
