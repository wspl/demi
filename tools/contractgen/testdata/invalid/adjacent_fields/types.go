package invalid

// +demi:root
// +demi:union tag=op content=result
type Reply interface{ seal() }

// +demi:variant Reply yes
type Broken struct {
	A string `json:"a"`
	B string `json:"b"`
}
