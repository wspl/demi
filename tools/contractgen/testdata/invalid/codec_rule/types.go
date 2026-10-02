package invalid

// +demi:root
// +demi:codec
// +demi:length min=1
type Broken string

func (Broken) MarshalJSON() ([]byte, error) { return nil, nil }
func (*Broken) UnmarshalJSON([]byte) error  { return nil }
