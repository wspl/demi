package browser

import (
	"bytes"
	"context"
	"image/png"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/page"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/cmdsdk"
)

func screenshot(
	ctx context.Context,
	tab *tabs.Tab,
	invocation *cmdsdk.InvocationContext[cmdproto.Invocation],
	input *browserproto.ScreenshotInput,
) (commandOutput, error) {
	if input.Output == nil && invocation.Request.JSON != nil && *invocation.Request.JSON {
		return commandOutput{}, &cdp.BrowserError{
			Kind:    cdp.KindConfiguration,
			Message: "screenshot --json requires --output",
		}
	}
	overwrite := input.Overwrite != nil && *input.Overwrite
	if input.Output != nil {
		if _, err := cdp.Preflight(ctx, invocation.Request.Cwd, *input.Output, overwrite); err != nil {
			return commandOutput{}, err
		}
	}
	data, err := page.Capture(ctx, tab, input.FullPage != nil && *input.FullPage, input.Clip, input.Timeout())
	if err != nil {
		return commandOutput{}, err
	}
	if input.Output == nil {
		return commandOutput{png: data}, nil
	}
	config, err := png.DecodeConfig(bytes.NewReader(data))
	if err != nil {
		return commandOutput{}, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: err.Error(), Cause: err}
	}
	metadata, err := page.Metadata(ctx, tab, input.Timeout())
	if err != nil {
		return commandOutput{}, err
	}
	path, err := cdp.SaveWithOverwrite(ctx, invocation.Request.Cwd, *input.Output, data, overwrite)
	return resultOutput(
		browserproto.ScreenshotResult{
			Path:     path,
			MIMEType: "image/png",
			Width:    uint32(config.Width),
			Height:   uint32(config.Height),
			Viewport: metadata.Viewport,
		},
		err,
	)
}

func probe(
	ctx context.Context,
	tab *tabs.Tab,
	invocation *cmdsdk.InvocationContext[cmdproto.Invocation],
	input *browserproto.ProbeInput,
	deadline time.Time,
) (commandOutput, error) {
	overwrite := input.Overwrite != nil && *input.Overwrite
	if _, err := cdp.Preflight(ctx, invocation.Request.Cwd, *input.Output, overwrite); err != nil {
		return commandOutput{}, err
	}
	checkout := tab.Gate().TryCheckout()
	if checkout == nil {
		return commandOutput{}, &cdp.BrowserError{Kind: cdp.KindBusy}
	}
	defer checkout.Release()
	raw, err := page.CommandAdmitted(ctx, tab, input, deadline, &checkout.Session().References)
	if err != nil {
		return commandOutput{}, err
	}
	result, err := browserproto.DecodeProbeResult(raw)
	if err != nil {
		return commandOutput{}, err
	}
	operation := tab.Operation(ctx, deadline)
	defer operation.Close()
	var data []byte
	err = operation.Run(ctx, func(work context.Context) error {
		var err error
		data, err = page.ScreenshotBytes(work, tab, false, nil)
		return err
	})
	if err != nil {
		return commandOutput{}, err
	}
	data, err = page.AnnotateProbe(data, result)
	if err != nil {
		return commandOutput{}, err
	}
	path, err := cdp.SaveWithOverwrite(ctx, invocation.Request.Cwd, *input.Output, data, overwrite)
	result.Path = &path
	return resultOutput(result, err)
}
