package server_test

import (
	"fmt"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/agent/server/servertest"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func TestUploadedImageTravelsByReferenceAndLoadsInline(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		blobs := storetest.NewMemoryBlobs()
		png := storetest.PNG(4, 3, 1)
		blob, err := blobs.Put(t.Context(), png)
		if err != nil {
			t.Fatal(err)
		}
		blocks, media, err := store.UploadBlocks(
			t.Context(),
			store.Upload{
				Name:      "tiny.png",
				Path:      "/home/demi/.demi/attachments/conversation/tiny.png",
				MediaType: "image/png",
				SHA256:    blob,
				Bytes:     png,
			},
			blobs,
		)
		if err != nil {
			t.Fatal(err)
		}
		f := fixtureWith(
			t,
			providertest.NewScriptedRuntime(
				t,
				said("a tiny png"),
				said("still a png"),
				said("gone"),
				said("still gone"),
			),
			storetest.NewMemoryTreeStoreWithBlobs(blobs),
			server.DefaultConfig(),
		)
		f.resolver.Select(storetest.ModelReading("stub", "test-model", []core.FileExtension{core.FileExtensionPNG}))
		files := servertest.NewFiles()
		files.Upload("upload-1", blocks, media)
		c := servertest.ConnectWith(t, f.server, rootID(), "/workspace", files)
		c.Send(t.Context(), &framewire.OpenFrame{})
		c.Received()
		c.Send(
			t.Context(),
			&framewire.SendFrame{
				MessageID: turnID("m1"),
				Content: []framewire.ClientContent{
					&framewire.TextContent{Text: "describe this"},
					&framewire.UploadContent{Ref: "upload-1", FileName: "tiny.png"},
				},
			},
		)
		frames := untilIdle(t, c)
		want := append(storetest.Text("describe this"), blocks...)
		stored := f.store.Checkpoint(rootID()).Transcript[0].(*core.UserBlock)
		equal(t, want, stored.Content)
		found := false
		for _, frame := range frames {
			if p, ok := frame.(*framewire.TranscriptPatchFrame); ok {
				for _, p := range p.Patches {
					if a, ok := p.(*framewire.AddPatch); ok {
						if u, ok := a.Value.(*core.UserBlock); ok {
							equal(t, want, u.Content)
							found = true
						}
					}
				}
			}
		}
		if !found {
			t.Fatal("no user patch")
		}
		for i := 2; i <= 3; i++ {
			c.Send(t.Context(), &framewire.CloseFrame{})
			c.Received()
			if i == 3 {
				blobs.Forget(blob)
			}
			c.Send(t.Context(), &framewire.OpenFrame{})
			c.Received()
			c.Send(t.Context(), send(fmt.Sprintf("m%d", i), "and now?"))
			untilIdle(t, c)
		}
		if _, err := blobs.Put(t.Context(), png); err != nil {
			t.Fatal(err)
		}
		c.Send(t.Context(), send("m4", "and now?"))
		untilIdle(t, c)
		requests := f.script.Requests()
		equal(t, 4, len(requests))
		for i, r := range requests {
			parts := r.Items[0].(*provider.UserMessage).Content
			equal(t, 3, len(parts))
			equal(t, provider.UserPart(&provider.TextPart{Text: "describe this"}), parts[0])
			equal(
				t,
				provider.UserPart(
					&provider.TextPart{Text: core.AttachmentTag(blocks[1].(*core.UserAttachment).Attachment)},
				),
				parts[2],
			)
			if i < 2 {
				equal(
					t,
					provider.UserPart(
						&provider.ImagePart{Medium: &provider.MediaBytes{Data: png, MediaType: "image/png"}},
					),
					parts[1],
				)
			} else {
				equal(t, provider.UserPart(&provider.TextPart{Text: "[missing image]"}), parts[1])
			}
		}
	})
}

func TestEditKeepsOnlyFilesHeldByItsMessage(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		blobs := storetest.NewMemoryBlobs()
		f := fixtureWith(
			t,
			providertest.NewScriptedRuntime(t, said("a chart"), said("same chart")),
			storetest.NewMemoryTreeStoreWithBlobs(blobs),
			server.DefaultConfig(),
		)
		f.resolver.Select(
			storetest.ModelReading(
				"stub",
				"test-model",
				[]core.FileExtension{core.FileExtensionPNG, core.FileExtensionMP4, core.FileExtensionPDF},
			),
		)
		media := store.HeldMedia{}
		refs := []core.BlobRef{}
		for _, data := range []core.B64Bytes{
			[]byte{0x89, 'P', 'N', 'G'},
			[]byte("\x00\x00\x00\x18ftypmp42"),
			[]byte("%PDF-1.7"),
		} {
			ref, err := blobs.Put(t.Context(), data)
			if err != nil {
				t.Fatal(err)
			}
			refs = append(refs, ref)
			media.Hold(ref, data)
		}
		attachment := &core.UserAttachment{
			Attachment: core.Attachment{
				Name:      "chart.png",
				Path:      "/home/demi/.demi/attachments/conversation/chart.png",
				MediaType: "image/png",
				SizeBytes: 4,
				SHA256:    refs[0],
			},
		}
		blocks := []core.UserContentBlock{
			&core.UserImage{Source: &core.MediaSourceRef{Ref: refs[0], MediaType: "image/png"}},
			&core.UserVideo{Source: &core.MediaSourceRef{Ref: refs[1], MediaType: "video/mp4"}},
			&core.UserDocument{
				Source: &core.DocumentRef{Ref: refs[2], MediaType: "application/pdf", FileName: "paper.pdf"},
			},
			attachment,
		}
		files := servertest.NewFiles()
		files.Upload("files", blocks, media)
		c := servertest.ConnectWith(t, f.server, rootID(), "/workspace", files)
		c.Send(t.Context(), &framewire.OpenFrame{})
		c.Received()
		c.Send(
			t.Context(),
			&framewire.SendFrame{
				MessageID: turnID("m1"),
				Content:   []framewire.ClientContent{&framewire.UploadContent{Ref: "files", FileName: "chart.png"}},
			},
		)
		untilIdle(t, c)
		target := userBlock(t, f, "m1")
		version := f.server.Tree(rootID()).Root().Session().Transcript().Version
		elsewhere, err := blobs.Put(t.Context(), []byte("GIF89a"))
		if err != nil {
			t.Fatal(err)
		}
		unknown := []framewire.ClientContent{
			&framewire.AttachmentContent{Path: "/elsewhere/chart.png"},
			&framewire.MediaContent{Media: &framewire.MediaImageRef{Ref: elsewhere, MediaType: "image/png"}},
			&framewire.MediaContent{Media: &framewire.MediaVideoRef{Ref: refs[0], MediaType: "video/mp4"}},
		}
		for i, content := range unknown {
			r := edit(fmt.Sprintf("op%d", i), target, version, "")
			r.Request.Content = []framewire.ClientContent{content}
			c.Send(t.Context(), r)
			rejectedEdit(
				t,
				editOutcome(t, c.Received()),
				[]string{
					"The edited message holds no attachment at /elsewhere/chart.png",
					fmt.Sprintf("The edited message holds no image %s", elsewhere),
					fmt.Sprintf("The edited message holds no video %s", refs[0]),
				}[i],
			)
			synctest.Wait()
			c.Received()
		}
		r := edit("op4", target, version, "look again")
		r.Request.Content = append(
			r.Request.Content,
			&framewire.MediaContent{Media: &framewire.MediaImageRef{Ref: refs[0], MediaType: "image/png"}},
			&framewire.MediaContent{Media: &framewire.MediaVideoRef{Ref: refs[1], MediaType: "video/mp4"}},
			&framewire.MediaContent{
				Media: &framewire.MediaDocumentRef{Ref: refs[2], MediaType: "text/plain", FileName: "renamed.txt"},
			},
			&framewire.AttachmentContent{Path: attachment.Path},
		)
		c.Send(t.Context(), r)
		if _, ok := editOutcome(t, untilIdle(t, c)).(*framewire.AcceptedEdit); !ok {
			t.Fatal("held files refused")
		}
		equal(
			t,
			append(storetest.Text("look again"), blocks...),
			f.store.Checkpoint(rootID()).Transcript[0].(*core.UserBlock).Content,
		)
		parts := f.script.Requests()[1].Items[0].(*provider.UserMessage).Content
		equal(
			t,
			[]provider.UserPart{
				&provider.TextPart{Text: "look again"},
				&provider.ImagePart{
					Medium: &provider.MediaBytes{Data: []byte{0x89, 'P', 'N', 'G'}, MediaType: "image/png"},
				},
				&provider.VideoPart{
					Medium: &provider.MediaBytes{Data: []byte("\x00\x00\x00\x18ftypmp42"), MediaType: "video/mp4"},
				},
				&provider.DocumentPart{
					Bytes:    provider.MediaBytes{Data: []byte("%PDF-1.7"), MediaType: "application/pdf"},
					FileName: "paper.pdf",
				},
				&provider.TextPart{Text: core.AttachmentTag(attachment.Attachment)},
			},
			parts,
		)
	})
}
