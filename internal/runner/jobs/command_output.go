package jobs

import (
	"context"
	"errors"
	"fmt"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/declare"
)

// commandOutput withholds a JSON command's stdout until its successful validation.
type commandOutput struct {
	output *cmdsdk.Output
	schema *declare.Schema
	bytes  []byte
}

func (o *commandOutput) Stdout(ctx context.Context, bytes []byte) error {
	if o.schema == nil {
		return o.output.Stdout(ctx, bytes)
	}
	if len(o.bytes)+len(bytes) > 1024*1024 {
		return errors.New("--json output exceeds 1 MiB")
	}
	o.bytes = append(o.bytes, bytes...)
	return nil
}

func (o *commandOutput) Stderr(ctx context.Context, bytes []byte) error {
	return o.output.Stderr(ctx, bytes)
}

func (o *commandOutput) finish(ctx context.Context, code uint8) error {
	if code != 0 || o.schema == nil {
		return nil
	}
	if err := contract.CheckJSON(o.bytes); err != nil {
		return fmt.Errorf("--json output is not JSON: %w", err)
	}
	if err := o.schema.Check(o.bytes); err != nil {
		return fmt.Errorf("--json output does not match its schema: %w", err)
	}
	return o.output.Stdout(ctx, o.bytes)
}
