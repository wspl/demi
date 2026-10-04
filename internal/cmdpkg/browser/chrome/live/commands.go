package live

import (
	"context"
	"log/slog"
	"sync"

	"github.com/chromedp/cdproto/page"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

func runCommands(
	ctx context.Context,
	environment *tabs.Environment,
	member *membership,
	w *writer,
	commands <-chan *browserproto.LiveViewerMessageMode,
) {
	for {
		if ctx.Err() != nil {
			return
		}
		var command *browserproto.LiveViewerMessageMode
		select {
		case <-ctx.Done():
			return
		case command = <-commands:
		}
		tab, err := environment.Tab(ctx, command.Tab, cdp.ControlTimeout)
		if err == nil {
			wasPhone := tab.Viewport().Mode == "mobile"
			err = member.mode(ctx, tab, browserproto.ViewportMode(command.Mode))
			if err == nil && wasPhone != (command.Mode == "mobile") {
				tab.Reload()
			}
		}
		if err != nil && ctx.Err() == nil {
			w.notice(ctx, string(cdp.ErrorCode(err)), err.Error())
		}
	}
}

func answerDialog(ctx context.Context, tab *tabs.Tab, accept bool, text *string, w *writer, answers *sync.WaitGroup) {
	answers.Add(1)
	err := tab.StartTask(func(owner context.Context) {
		defer answers.Done()
		// The caller joins this task before retiring its output writer.
		work, cancel := context.WithCancel(ctx)
		defer cancel()
		stop := context.AfterFunc(owner, cancel)
		defer stop()
		dialog := tab.Dialog().Open()
		if dialog == nil {
			return
		}
		if dialog.Type != page.DialogTypePrompt || !accept {
			text = nil
		}
		if err := tab.Dialog().Answer(work, dialog, accept, text); err != nil && tab.Dialog().IsOpen() {
			w.notice(work, string(cdp.ErrorCode(err)), err.Error())
		}
	})
	if err != nil {
		answers.Done()
		slog.Debug("live dialog answer admission", "error", err)
	}
}
