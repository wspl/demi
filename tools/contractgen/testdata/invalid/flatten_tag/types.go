package invalid

// +demi:root
type Broken struct {
	// +demi:flatten
	Result Reply `json:"result"`
}

// +demi:union tag=op content=result
type Reply interface{ seal() }

// +demi:variant Reply yes
type Yes struct{}
