package hostaccess

//go:generate go run ../../../tools/contractgen

// The input of a leaf that takes none, such as `demi host list`.
// +demi:schema
// +demi:root
type noArgs struct{}

// The input of `demi host shell`.
// +demi:schema
// +demi:root
type shellArgs struct {
	// Host name or device id from demi host list
	Host string `json:"host"`
	// One quoted shell script argument; stdin is streamed to the remote
	// program, not read as script text
	Script string `json:"script"`
}
