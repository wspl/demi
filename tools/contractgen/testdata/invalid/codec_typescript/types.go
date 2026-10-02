package invalid

import "github.com/wspl/demi/internal/runnerwire"

// +demi:root direction=receive output=web
type Broken struct {
	URL runnerwire.BackendURL `json:"url"`
}
