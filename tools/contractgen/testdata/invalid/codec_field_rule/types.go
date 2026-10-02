package invalid

// +demi:root
type Broken struct {
	// +demi:length min=1
	Value **Private `json:"value"`
}

// +demi:codec
type Private string

func (Private) MarshalJSON() ([]byte, error) { return nil, nil }
func (*Private) UnmarshalJSON([]byte) error  { return nil }
