package invalid

import "github.com/wspl/demi/internal/runnerwire"

// +demi:root
// +demi:schema
type Broken struct {
	Boot runnerwire.ManagedBoot `json:"boot"`
}
