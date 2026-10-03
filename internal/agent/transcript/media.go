package transcript

import (
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

// userPart renders a message part using the request's single media policy.
func (r *RequestView) userPart(part core.UserContentBlock) provider.UserPart {
	switch p := part.(type) {
	case *core.UserText:
		return &provider.TextPart{Text: boundText(p.Text)}
	case *core.UserReference:
		return &provider.TextPart{Text: p.Reference}
	case *core.UserAttachment:
		return &provider.TextPart{Text: core.AttachmentTag(p.Attachment)}
	case *core.UserImage:
		medium, text := r.medium("image", p.Source)
		if medium == nil {
			return &provider.TextPart{Text: text}
		}
		return &provider.ImagePart{Medium: medium}
	case *core.UserVideo:
		medium, text := r.medium("video", p.Source)
		if medium == nil {
			return &provider.TextPart{Text: text}
		}
		return &provider.VideoPart{Medium: medium}
	case *core.UserDocument:
		switch source := p.Source.(type) {
		case *core.DocumentRef:
			data, text := r.document(source)
			if data == nil {
				return &provider.TextPart{Text: text}
			}
			return &provider.DocumentPart{Bytes: *data, FileName: source.FileName}
		}
	}
	return nil
}

// medium renders a message's image or video, retaining URLs for the vendor.
func (r *RequestView) medium(kind string, source core.MediaSource) (provider.Medium, string) {
	switch s := source.(type) {
	case *core.MediaURL:
		return &provider.MediaURL{URL: s.URL}, ""
	case *core.MediaSourceRef:
		data, text := r.mediaBytes(
			kind,
			s.Ref,
			s.MediaType,
			s.MediaType,
			core.ModelAcceptsMediaType(r.model, s.MediaType),
		)
		if data == nil {
			return nil, text
		}
		return data, ""
	}
	return nil, ""
}

// document resolves a document using the model's native PDF support.
func (r *RequestView) document(source *core.DocumentRef) (*provider.MediaBytes, string) {
	mediaType, _, _ := strings.Cut(source.MediaType, ";")
	accepted := strings.TrimSpace(mediaType) == "application/pdf" &&
		core.AcceptsFileExtension(r.model.AcceptedExtensions, core.FileExtensionPDF)
	return r.mediaBytes("document", source.Ref, source.MediaType, source.FileName, accepted)
}

// toolMedium resolves a stored tool medium through the same request policy.
func (r *RequestView) toolMedium(kind string, source core.ToolMediaSource) (*provider.MediaBytes, string) {
	switch s := source.(type) {
	case *core.ToolMediaRef:
		return r.mediaBytes(kind, s.Ref, s.MediaType, s.MediaType, core.ModelAcceptsMediaType(r.model, s.MediaType))
	}
	return nil, ""
}

// mediaBytes resolves held media or its stable model-facing refusal text.
func (r *RequestView) mediaBytes(
	kind string,
	blob core.BlobRef,
	mediaType, name string,
	accepted bool,
) (*provider.MediaBytes, string) {
	var data core.B64Bytes
	switch held := r.view.Held(blob).(type) {
	case *store.HeldMissing:
		return nil, store.MissingText(kind)
	case *store.HeldBytes:
		data = held.Bytes
	}
	reason := ""
	if !accepted {
		reason = "the model does not accept it"
	} else if r.halfBody != nil && data.Base64Len() > *r.halfBody {
		reason = "too large for the model's requests"
	}
	if reason != "" {
		return nil, fmt.Sprintf("[%s:%s, not sent: %s]", kind, name, reason)
	}
	return &provider.MediaBytes{Data: data, MediaType: mediaType}, ""
}

// result renders one tool output part, including the text of retired media.
func (r *RequestView) result(part core.ToolResultContentBlock) provider.ResultPart {
	switch p := part.(type) {
	case *core.ToolText:
		return &provider.TextPart{Text: boundText(p.Text)}
	case *core.ToolGone:
		return &provider.TextPart{Text: boundText(goneText(p))}
	case *core.ToolImage:
		data, text := r.toolMedium("image", p.Source)
		if data == nil {
			return &provider.TextPart{Text: text}
		}
		return &provider.ResultImage{Bytes: *data}
	case *core.ToolVideo:
		data, text := r.toolMedium("video", p.Source)
		if data == nil {
			return &provider.TextPart{Text: text}
		}
		return &provider.ResultVideo{Bytes: *data}
	}
	return nil
}

// goneText names a tool medium that could not be stored or was retired.
func goneText(part *core.ToolGone) string {
	switch cause := part.Cause.(type) {
	case *core.NotStored:
		return fmt.Sprintf("[%s not stored: %s]", part.Kind, cause.Error)
	case *core.Retired:
		// Timestamps were validated at the transcript's entry boundary.
		day, _, _ := strings.Cut(string(cause.At), "T")
		return fmt.Sprintf(
			"[%s:%s, removed on %s: a tool result's images and videos are kept for 30 days]",
			part.Kind,
			part.MediaType,
			day,
		)
	}
	return ""
}
