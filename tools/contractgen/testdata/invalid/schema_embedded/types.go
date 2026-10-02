package invalid

// +demi:schema
// +demi:root direction=receive
type Broken struct{ Checked }

// +demi:check validate
type Checked struct {
	Value string `json:"value"`
}

func validate(Checked) error { return nil }
