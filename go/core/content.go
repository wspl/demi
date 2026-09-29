package core

// The record of a file that came with a message: on the conversation's Host
// at `path`, never inlined. The model reads it as a tag that names the file
// ([`attachment_tag`]); the page draws the file's tile from it.
//
//demi:wire
type Attachment struct {
	Name string `json:"name" check:"chars=1.."`
	// Absolute, on the conversation's Host.
	Path      string `json:"path" check:"chars=1.."`
	MediaType string `json:"mediaType"`
	SizeBytes uint64 `json:"sizeBytes" check:"range=..MaxSafeInteger"`
	// The uploaded bytes in the blob store, from which the page fetches
	// them.
	SHA256 BlobRef `json:"sha256" check:"func=Validate"`
	// The opening of a text file, for the tile that shows it as a page.
	Snippet *string `json:"snippet,omitzero"`
}

// One part of a message or a steer, in the transcript and in the queue.
//
//demi:union tag=type
//demi:export
type UserContentBlock interface{ isUserContentBlock() }

//demi:variant text
type UserContentBlockText struct {
	Text string `json:"text"`
}

func (UserContentBlockText) isUserContentBlock() {}

// An image the model reads natively.
//
//demi:variant image
type UserContentBlockImage struct {
	Source MediaSource `json:"source"`
}

func (UserContentBlockImage) isUserContentBlock() {}

// A video the model reads natively; only a model whose catalog marks
// video support receives one.
//
//demi:variant video
type UserContentBlockVideo struct {
	Source MediaSource `json:"source"`
}

func (UserContentBlockVideo) isUserContentBlock() {}

// A PDF the model reads natively.
//
//demi:variant document
type UserContentBlockDocument struct {
	Source DocumentSource `json:"source"`
}

func (UserContentBlockDocument) isUserContentBlock() {}

// Text that names something, such as a file on a paired device and the
// command that reads it; the model reads it as text.
//
//demi:variant reference
type UserContentBlockReference struct {
	Reference string `json:"reference"`
}

func (UserContentBlockReference) isUserContentBlock() {}

// A file that came with the message.
//
//demi:variant attachment
type UserContentBlockAttachment struct {
	Attachment
}

func (UserContentBlockAttachment) isUserContentBlock() {}

// Where a message's image or video is: never its bytes, which a session
// holds beside the transcript while a request can send them (`runtime.md`
// § Media).
//
//demi:union tag=type
//demi:export
type MediaSource interface{ isMediaSource() }

// A URL the provider fetches.
//
//demi:variant url
type MediaSourceURL struct {
	URL string `json:"url"`
}

func (MediaSourceURL) isMediaSource() {}

// The bytes in the conversation owner's blob namespace, as a store keeps
// them and as the page receives them.
//
//demi:variant ref
type MediaSourceRef struct {
	Ref       BlobRef `json:"ref" check:"func=Validate"`
	MediaType string  `json:"mediaType"`
}

func (MediaSourceRef) isMediaSource() {}

// Where a document's bytes are, with the name the file came with: in the
// conversation owner's blob namespace. A tagged enum of one variant, so a
// stored document keeps `{ "type": "ref", ... }`.
//
//demi:union tag=type
//demi:export
type DocumentSource interface{ isDocumentSource() }

//demi:variant ref
type DocumentSourceRef struct {
	Ref       BlobRef `json:"ref" check:"func=Validate"`
	MediaType string  `json:"mediaType"`
	FileName  string  `json:"fileName"`
}

func (DocumentSourceRef) isDocumentSource() {}

// One part of a tool's result.
//
//demi:union tag=type
//demi:export
type ToolResultContentBlock interface{ isToolResultContentBlock() }

//demi:variant text
type ToolResultContentBlockText struct {
	Text string `json:"text"`
}

func (ToolResultContentBlockText) isToolResultContentBlock() {}

//demi:variant image
type ToolResultContentBlockImage struct {
	Source ToolMediaSource `json:"source"`
}

func (ToolResultContentBlockImage) isToolResultContentBlock() {}

//demi:variant video
type ToolResultContentBlockVideo struct {
	Source ToolMediaSource `json:"source"`
}

func (ToolResultContentBlockVideo) isToolResultContentBlock() {}

// An image or a video the result no longer holds, in its place: what
// it was and why it is gone (`runtime.md` § Media). The model reads it
// as one line of text.
//
//demi:variant gone
type ToolResultContentBlockGone struct {
	Kind      ModelMediaKind `json:"kind"`
	MediaType string         `json:"mediaType"`
	Cause     GoneCause      `json:"cause"`
}

func (ToolResultContentBlockGone) isToolResultContentBlock() {}

// Why a tool result's image or video is gone.
//
//demi:union tag=type
//demi:export
type GoneCause interface{ isGoneCause() }

// Its bytes could not be stored when the result entered the
// transcript; `error` is the store's.
//
//demi:variant not_stored
type GoneCauseNotStored struct {
	Error string `json:"error"`
}

func (GoneCauseNotStored) isGoneCause() {}

// It was retired at `at`, 30 days on (`runtime.md` § Retired tool
// media).
//
//demi:variant retired
type GoneCauseRetired struct {
	At Timestamp `json:"at" check:"func=Validate"`
}

func (GoneCauseRetired) isGoneCause() {}

// Where the bytes of a tool result's image or video are: in the
// conversation owner's blob namespace. A tagged enum of one variant, so a
// stored result keeps `{ "type": "ref", ... }`.
//
//demi:union tag=type
//demi:export
type ToolMediaSource interface{ isToolMediaSource() }

//demi:variant ref
type ToolMediaSourceRef struct {
	Ref       BlobRef `json:"ref" check:"func=Validate"`
	MediaType string  `json:"mediaType"`
}

func (ToolMediaSourceRef) isToolMediaSource() {}
