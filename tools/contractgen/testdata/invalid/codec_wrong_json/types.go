package invalid

// +demi:root
// +demi:codec
type Broken struct{}

func (Broken) MarshalJSON() ([]byte, error) { return nil, nil }
func (*Broken) UnmarshalJSON(string) error  { return nil }
