package webapi

import (
	"github.com/wspl/demi/go/agentproto"
	"github.com/wspl/demi/go/core"
)

// `PUT /conversations/:id/draft`: the message's Markdown and its files in
// the order of their marks, each named as a frame names it, an `upload` or
// a `remote_file`.
//
//demi:wire
type DraftSave struct {
	// The revision the page's text was built on.
	Base  uint64                     `json:"base" check:"range=..9007199254740991"`
	Text  string                     `json:"text"`
	Files []agentproto.ClientContent `json:"files" check:"each(func=agentproto.Validate)"`
}

// What the page asks of the version a save replaced.
//
//demi:enum
//demi:export
type ReplacedAction string

const (
	// Exchange it with the draft, which then becomes the replaced version.
	ReplacedActionRestore ReplacedAction = "restore"
	// Drop it.
	ReplacedActionDismiss ReplacedAction = "dismiss"
)

// `POST /conversations/:id/draft/replaced`: an action on the replaced
// version the page shows, named by its revision.
//
//demi:wire
type ReplacedDraftAction struct {
	Action   ReplacedAction `json:"action"`
	Revision uint64         `json:"revision" check:"range=..9007199254740991"`
}

// The answer of every draft route: the draft as it now is.
//
//demi:wire open
type DraftAnswer struct {
	Draft ConversationDraft `json:"draft"`
}

// A conversation's draft.
//
//demi:wire open
type ConversationDraft struct {
	// How many times the draft changed: 0 before its first save, one more
	// with every save, restore and dismissal.
	Revision uint64 `json:"revision" check:"range=..9007199254740991"`
	// The message's Markdown, with an attachment mark where each file's
	// capsule stands.
	Text string `json:"text"`
	// The files in the order of their marks.
	Files []DraftFile `json:"files"`
	// The version a save replaced without having been built on it.
	Replaced *ReplacedDraft `json:"replaced" check:"nullable"`
}

// A version of the draft that a save replaced, with the revision it had as
// the draft.
//
//demi:wire open
type ReplacedDraft struct {
	Revision uint64      `json:"revision" check:"range=..9007199254740991"`
	Text     string      `json:"text"`
	Files    []DraftFile `json:"files"`
}

// A file of a draft as every page shows it.
//
//demi:union tag=type
//demi:export
type DraftFile interface{ isDraftFile() }

// An upload, with what its record holds: the media type the backend
// read, where its bytes are in the caller's blobs, and a text file's
// opening.
//
//demi:variant upload open
type DraftFileUpload struct {
	Ref       AttachmentID `json:"ref" check:"func=Validate"`
	FileName  string       `json:"fileName"`
	MediaType string       `json:"mediaType"`
	SHA256    core.BlobRef `json:"sha256" check:"func=core.Validate"`
	Snippet   *string      `json:"snippet,omitzero"`
}

func (DraftFileUpload) isDraftFile() {}

// A file on a paired device, which the model reads when it runs.
//
//demi:variant remote_file open
type DraftFileRemoteFile struct {
	DeviceID string `json:"deviceId"`
	Path     string `json:"path"`
}

func (DraftFileRemoteFile) isDraftFile() {}
