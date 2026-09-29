package packtest

import "github.com/wspl/demi/go/internal/wiregen/featuretest"

//demi:wire
//demi:export
//demi:msgpack
type Normalized struct {
	Label featuretest.Trimmed `json:"label" check:"chars=1..3,func=featuretest.ValidateTrimmed"`
}
