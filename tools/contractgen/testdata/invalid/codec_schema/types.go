package invalid

import "github.com/wspl/demi/internal/runnerproto"

// +demi:root
// +demi:schema
type Broken struct {
	Boot runnerproto.ManagedBoot `json:"boot"`
}
