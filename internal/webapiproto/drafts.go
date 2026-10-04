package webapiproto

import (
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/types"
)

// The most bytes a draft's text and files take as stored.
const DraftBytesMax = 256 * 1024

// `PUT /conversations/:id/draft`: the message's Markdown and its files in
// the order of their marks, each named as a frame names it, an `upload` or
// a `remote_file`.
// +demi:root direction=send output=web
type DraftSave struct {
	// The revision the page's text was built on.
	// +demi:range max=9007199254740991
	Base  uint64                            `json:"base"`
	Text  string                            `json:"text"`
	Files []conversationproto.ClientContent `json:"files"`
}

// What the page asks of the version a save replaced.
// +demi:enum restore dismiss
type ReplacedAction string

// Values of the preceding enumeration.
const (
	// Exchange it with the draft, which then becomes the replaced version.
	ReplacedActionRestore ReplacedAction = "restore"
	// Drop it.
	ReplacedActionDismiss ReplacedAction = "dismiss"
)

// `POST /conversations/:id/draft/replaced`: an action on the replaced
// version the page shows, named by its revision.
// +demi:root direction=send output=web
type ReplacedDraftAction struct {
	Action ReplacedAction `json:"action"`
	// +demi:range max=9007199254740991
	Revision uint64 `json:"revision"`
}

// The answer of every draft route: the draft as it now is.
// +demi:root direction=receive output=web
// +demi:tolerant
type DraftAnswer struct {
	Draft ConversationDraft `json:"draft"`
}

// A conversation's draft.
// +demi:tolerant
type ConversationDraft struct {
	// How many times the draft changed: 0 before its first save, one more
	// with every save, restore and dismissal.
	// +demi:range max=9007199254740991
	Revision uint64 `json:"revision"`
	// The message's Markdown, with an attachment mark where each file's
	// capsule stands.
	Text string `json:"text"`
	// The files in the order of their marks.
	Files []DraftFile `json:"files"`
	// The version a save replaced without having been built on it.
	// +demi:nullable
	Replaced *ReplacedDraft `json:"replaced"`
}

// A version of the draft that a save replaced, with the revision it had as
// the draft.
// +demi:tolerant
type ReplacedDraft struct {
	// +demi:range max=9007199254740991
	Revision uint64      `json:"revision"`
	Text     string      `json:"text"`
	Files    []DraftFile `json:"files"`
}

// A file of a draft as every page shows it.
// +demi:union tag=type
//
//sumtype:decl
type DraftFile interface{ draftFile() }

// An upload, with what its record holds: the media type the backend
// read, where its bytes are in the caller's blobs, and a text file's
// opening.
// +demi:variant DraftFile upload
// +demi:tolerant
type DraftFileUpload struct {
	Ref       AttachmentID  `json:"ref"`
	FileName  string        `json:"fileName"`
	MediaType string        `json:"mediaType"`
	Sha256    types.BlobRef `json:"sha256"`
	Snippet   *string       `json:"snippet,omitempty"`
}

// A file on a paired device, which the model reads when it runs.
// +demi:variant DraftFile remote_file
// +demi:tolerant
type DraftFileRemoteFile struct {
	DeviceID string `json:"deviceId"`
	Path     string `json:"path"`
}

// The mark a draft's text holds where each file's capsule stands, the one
// the composer's editor writes (`product.md` § Attachments).
const AttachmentMark = '\uFFFC'

// The draft of a conversation that never saved one.
func EmptyConversationDraft() ConversationDraft { return ConversationDraft{Files: []DraftFile{}} }
