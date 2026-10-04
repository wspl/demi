// Package external exercises custom codecs owned by another contract package.
package external

import "github.com/wspl/demi/internal/webapiproto"

//go:generate go run ../..

// +demi:root direction=receive output=plugin-external
// +demi:schema
type Expose struct {
	Address webapiproto.ExposeAddress `json:"address"`
}
