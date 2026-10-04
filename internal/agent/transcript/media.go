package transcript

import (
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

// userPart renders a message part using the request's single media policy.
func (r *RequestView) userPart(part types.UserContentBlock) provider.UserPart {
	switch p := part.(type) {
	case *types.UserText:
		return &provider.TextPart{Text: boundText(p.Text)}
	case *types.UserReference:
		return &provider.TextPart{Text: p.Reference}
	case *types.UserAttachment:
		return &provider.TextPart{Text: types.AttachmentTag(p.Attachment)}
	case *types.UserImage:
		medium, text := r.medium("image", p.Source)
		if medium == nil {
			return &provider.TextPart{Text: text}
		}
		return &provider.ImagePart{Medium: medium}
	case *types.UserVideo:
		medium, text := r.medium("video", p.Source)
		if medium == nil {
			return &provider.TextPart{Text: text}
		}
		return &provider.VideoPart{Medium: medium}
	case *types.UserDocument:
		switch source := p.Source.(type) {
		case *types.DocumentRef:
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
func (r *RequestView) medium(kind string, source types.MediaSource) (provider.Medium, string) {
	switch s := source.(type) {
	case *types.MediaURL:
		return &provider.MediaURL{URL: s.URL}, ""
	case *types.MediaSourceRef:
		data, text := r.mediaBytes(
			kind,
			s.Ref,
			s.MediaType,
			s.MediaType,
			types.ModelAcceptsMediaType(r.model, s.MediaType),
		)
		if data == nil {
			return nil, text
		}
		return data, ""
	}
	return nil, ""
}

// document resolves a document using the model's native PDF support.
func (r *RequestView) document(source *types.DocumentRef) (*provider.MediaBytes, string) {
	mediaType, _, _ := strings.Cut(source.MediaType, ";")
	accepted := strings.TrimSpace(mediaType) == "application/pdf" &&
		types.AcceptsFileExtension(r.model.AcceptedExtensions, types.FileExtensionPDF)
	return r.mediaBytes("document", source.Ref, source.MediaType, source.FileName, accepted)
}

// toolMedium resolves a stored tool medium through the same request policy.
func (r *RequestView) toolMedium(kind string, source types.ToolMediaSource) (*provider.MediaBytes, string) {
	switch s := source.(type) {
	case *types.ToolMediaRef:
		return r.mediaBytes(kind, s.Ref, s.MediaType, s.MediaType, types.ModelAcceptsMediaType(r.model, s.MediaType))
	}
	return nil, ""
}

// mediaBytes resolves held media or its stable model-facing refusal text.
func (r *RequestView) mediaBytes(
	kind string,
	blob types.BlobRef,
	mediaType, name string,
	accepted bool,
) (*provider.MediaBytes, string) {
	data, found := r.view.Held(blob)
	if !found {
		return nil, store.MissingText(kind)
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
func (r *RequestView) result(part types.ToolResultContentBlock) provider.ResultPart {
	switch p := part.(type) {
	case *types.ToolText:
		return &provider.TextPart{Text: boundText(p.Text)}
	case *types.ToolGone:
		return &provider.TextPart{Text: boundText(goneText(p))}
	case *types.ToolImage:
		data, text := r.toolMedium("image", p.Source)
		if data == nil {
			return &provider.TextPart{Text: text}
		}
		return &provider.ResultImage{Bytes: *data}
	case *types.ToolVideo:
		data, text := r.toolMedium("video", p.Source)
		if data == nil {
			return &provider.TextPart{Text: text}
		}
		return &provider.ResultVideo{Bytes: *data}
	}
	return nil
}

// goneText names a tool medium that could not be stored or was retired.
func goneText(part *types.ToolGone) string {
	switch cause := part.Cause.(type) {
	case *types.NotStored:
		return fmt.Sprintf("[%s not stored: %s]", part.Kind, cause.Error)
	case *types.Retired:
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
