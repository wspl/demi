package invalid

import "github.com/wspl/demi/internal/runnerproto"

// +demi:root
type Broken struct{ *runnerproto.BackendURL }
