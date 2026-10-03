package main

import (
	"context"
	"errors"
)

// dev needs Unix process groups to stop what it starts in order.
func (*application) dev(context.Context, devOptions) error {
	return errors.New("dev requires Unix process groups")
}
