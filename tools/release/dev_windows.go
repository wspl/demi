package main

import (
	"context"
	"errors"
)

type devProcess struct {
	done chan struct{}
	err  error
}

func (*application) serveDev(context.Context, devOptions, string, string) error {
	return errors.New("dev requires Unix process groups")
}
