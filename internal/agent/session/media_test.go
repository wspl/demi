package session_test

import (
	"context"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
)

func mediaParts(r provider.InferenceRequest) []string {
	parts := []string{}
	userFound := false
	for _, item := range r.Items {
		switch item := item.(type) {
		case *provider.UserMessage:
			if userFound {
				continue
			}
			userFound = true
			for _, p := range item.Content {
				switch p := p.(type) {
				case *provider.TextPart:
					parts = append(parts, p.Text)
				case *provider.ImagePart:
					parts = append(parts, "<image>")
				case *provider.VideoPart:
					parts = append(parts, "<video>")
				case *provider.DocumentPart:
					parts = append(parts, "<document>")
				}
			}
		case *provider.ToolResult:
			for _, p := range item.Output {
				switch p := p.(type) {
				case *provider.TextPart:
					parts = append(parts, p.Text)
				case *provider.ResultImage:
					parts = append(parts, "<image>")
				case *provider.ResultVideo:
					parts = append(parts, "<video>")
				}
			}
		case *provider.UserSteer,
			*provider.AssistantText,
			*provider.AssistantThinking,
			*provider.AssistantRedactedThinking,
			*provider.ToolUse:
		}
	}
	return parts
}

func TestUnsupportedMediaUsesStableTextAcrossModels(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		png := storetest.PNG(3, 2, 1)
		pdf := types.B64Bytes("%PDF-1.7")
		mp4 := make([]byte, 3000)
		copy(mp4, []byte("\x00\x00\x00\x18ftypisom"))
		r := toolRuntime("record", func(context.Context, session.ToolInvocation) (session.ToolOutcome, error) {
			return session.ToolOutcome{
				Output: []provider.ResultPart{
					&provider.ResultVideo{Bytes: provider.MediaBytes{Data: mp4, MediaType: "video/mp4"}},
				},
			}, nil
		})
		f := start(
			t,
			r,
			session.DefaultConfig(),
			tool("record"),
			answer("recorded"),
			answer("seen"),
			answer("seen again"),
			answer("back"),
		)
		a := storetest.ModelReading(
			"stub",
			"model-a",
			[]types.FileExtension{types.FileExtensionPNG, types.FileExtensionPDF, types.FileExtensionMP4},
		)
		b := storetest.ModelReading("stub", "model-b", []types.FileExtension{types.FileExtensionPNG})
		c := storetest.ModelReading(
			"small",
			"model-c",
			[]types.FileExtension{types.FileExtensionPNG, types.FileExtensionPDF, types.FileExtensionMP4},
		)
		held := store.HeldMedia{}
		held.Hold(types.BlobRefOf(png), png)
		held.Hold(types.BlobRefOf(pdf), pdf)
		f.s.HoldMedia(&held)
		must(t, f.s.UpdateModel(session.ModelSwitch{Model: a}))
		content := []types.UserContentBlock{
			&types.UserText{Text: "record it"},
			&types.UserImage{Source: &types.MediaSourceRef{Ref: types.BlobRefOf(png), MediaType: "image/png"}},
			&types.UserDocument{
				Source: &types.DocumentRef{
					Ref:       types.BlobRefOf(pdf),
					MediaType: "application/pdf",
					FileName:  "spec.pdf",
				},
			},
		}
		handle, err := f.s.Send(content, "t1")
		must(t, err)
		f.done(handle)
		must(t, f.s.UpdateModel(session.ModelSwitch{Model: b}))
		f.done(f.send("look", "t2"))
		f.done(f.send("look again", "t3"))
		small := providertest.NewScriptedRuntime(t, answer("small"))
		small.SetLimits(provider.RequestLimits{BodyBytes: new(uint64(6000))})
		must(t, f.s.UpdateModel(session.ModelSwitch{Model: c, Runtime: small}))
		f.done(f.send("small", "t4"))
		must(t, f.s.UpdateModel(session.ModelSwitch{Model: a, Runtime: f.p.Fresh()}))
		f.done(f.send("back", "t5"))
		req := f.p.Requests()
		sent := []string{"record it", "<image>", "<document>", "<video>"}
		equal(t, mediaParts(req[1]), sent)
		unread := []string{
			"record it",
			"<image>",
			"[document:spec.pdf, not sent: the model does not accept it]",
			"[video:video/mp4, not sent: the model does not accept it]",
		}
		equal(t, mediaParts(req[2]), unread)
		equal(t, mediaParts(req[3]), unread)
		equal(t, req[3].Items[:len(req[2].Items)], req[2].Items)
		equal(
			t,
			mediaParts(small.Requests()[0]),
			[]string{
				"record it",
				"<image>",
				"<document>",
				"[video:video/mp4, not sent: too large for the model's requests]",
			},
		)
		equal(t, mediaParts(req[4]), sent)
	})
}
