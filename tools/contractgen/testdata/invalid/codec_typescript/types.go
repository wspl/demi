package invalid

import "github.com/wspl/demi/internal/runnerproto"

// +demi:root direction=receive output=web
type Broken struct {
	URL runnerproto.BackendURL `json:"url"`
}
