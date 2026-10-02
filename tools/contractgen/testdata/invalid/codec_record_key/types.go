package invalid

// +demi:root
type Broken map[Key]string

// +demi:codec
type Key string

func (Key) MarshalJSON() ([]byte, error) { return nil, nil }
func (*Key) UnmarshalJSON([]byte) error  { return nil }
