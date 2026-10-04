package process_test

import (
	"context"
	"errors"
	"testing"

	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerproto"
)

func TestReportPipe(t *testing.T) {
	frames := make(chan []byte, 1)
	if err := process.ReportPipe(t.Context(), frames, "pipe", errors.New("write failed")); err != nil {
		t.Fatal(err)
	}
	report, err := runnerproto.DecodeOutbound(<-frames)
	if err != nil {
		t.Fatal(err)
	}
	done, ok := report.(*runnerproto.PipeDone)
	if !ok || done.PipeID != "pipe" || done.Ok || done.Error == nil || *done.Error != "write failed" {
		t.Fatalf("report %+v", report)
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	if err := process.ReportPipe(ctx, make(chan []byte), "pipe", nil); !errors.Is(err, context.Canceled) {
		t.Fatalf("report cancel: %v", err)
	}
}
