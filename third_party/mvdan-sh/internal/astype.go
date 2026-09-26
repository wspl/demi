// Copyright (c) 2026, Daniel Martí <mvdan@mvdan.cc>
// See LICENSE for licensing information

package internal

import "errors"

// AsType is errors.AsType from Go 1.26, for the Go version Demi's module pins.
func AsType[E error](err error) (E, bool) {
	var target E
	ok := errors.As(err, &target)
	return target, ok
}
