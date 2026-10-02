package invalid

// +demi:root
type Broken struct {
	*Child
	Name string `json:"name"`
}

type Child struct {
	Name string `json:"name"`
}
