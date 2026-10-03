package providertest

//go:generate go run github.com/wspl/demi/tools/contractgen

// TokenDocument is a secret document of a made-up family.
// +demi:root
type TokenDocument struct {
	Access  string `json:"access"`
	Refresh string `json:"refresh"`
}
