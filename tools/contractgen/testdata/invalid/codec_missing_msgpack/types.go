package invalid

// +demi:msgpack
// +demi:codec
type Broken struct{}

func (Broken) MarshalJSON() ([]byte, error)    { return nil, nil }
func (*Broken) UnmarshalJSON([]byte) error     { return nil }
func (Broken) MarshalMsgpack() ([]byte, error) { return nil, nil }
