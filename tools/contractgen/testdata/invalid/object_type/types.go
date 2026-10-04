package invalid

import "encoding/json"

var _ json.RawMessage

// +demi:object
// +demi:root
type Broken json.RawMessage
