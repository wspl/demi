// Package external exercises custom codecs owned by another contract package.
package external

import "github.com/wspl/demi/internal/webapi"

//go:generate go run ../..

// +demi:root
type Expose struct {
	Address webapi.ExposeAddress `json:"address"`
}
