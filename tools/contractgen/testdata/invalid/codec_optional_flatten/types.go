package invalid

import "github.com/wspl/demi/internal/runnerwire"

// +demi:root
type Broken struct{ *runnerwire.BackendURL }
