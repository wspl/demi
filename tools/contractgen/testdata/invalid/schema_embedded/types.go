package invalid

// +demi:schema
type Broken struct{ Checked }

// +demi:check validate
type Checked struct {
	Value string `json:"value"`
}

func validate(Checked) error { return nil }
