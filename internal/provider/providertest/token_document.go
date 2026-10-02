package providertest

//go:generate go run ../../../tools/contractgen

// A secret document of a made-up family.
// +demi:root
//
//nolint:revive // Contract documentation is product text copied verbatim from Rust.
type TokenDocument struct {
	Access  string `json:"access"`
	Refresh string `json:"refresh"`
}
