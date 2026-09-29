package core

//demi:wire
type Attachment struct {
	Name      string  `json:"name" check:"chars=1.."`
	Path      string  `json:"path" check:"chars=1.."`
	MediaType string  `json:"mediaType"`
	SizeBytes uint64  `json:"sizeBytes" check:"range=..MaxSafeInteger"`
	SHA256    BlobRef `json:"sha256" check:"func=Validate"`
	Snippet   *string `json:"snippet,omitzero"`
}

//demi:union tag=type
type UserContentBlock interface{ isUserContentBlock() }

//demi:variant text
type UserContentBlockText struct {
	Text string `json:"text"`
}

func (UserContentBlockText) isUserContentBlock() {}

//demi:variant image
type UserContentBlockImage struct {
	Source MediaSource `json:"source"`
}

func (UserContentBlockImage) isUserContentBlock() {}

//demi:variant video
type UserContentBlockVideo struct {
	Source MediaSource `json:"source"`
}

func (UserContentBlockVideo) isUserContentBlock() {}

//demi:variant document
type UserContentBlockDocument struct {
	Source DocumentSource `json:"source"`
}

func (UserContentBlockDocument) isUserContentBlock() {}

//demi:variant reference
type UserContentBlockReference struct {
	Reference string `json:"reference"`
}

func (UserContentBlockReference) isUserContentBlock() {}

//demi:variant attachment
type UserContentBlockAttachment struct {
	Attachment
}

func (UserContentBlockAttachment) isUserContentBlock() {}

//demi:union tag=type
type MediaSource interface{ isMediaSource() }

//demi:variant url
type MediaSourceURL struct {
	URL string `json:"url"`
}

func (MediaSourceURL) isMediaSource() {}

//demi:variant ref
type MediaSourceRef struct {
	Ref       BlobRef `json:"ref" check:"func=Validate"`
	MediaType string  `json:"mediaType"`
}

func (MediaSourceRef) isMediaSource() {}

//demi:union tag=type
type DocumentSource interface{ isDocumentSource() }

//demi:variant ref
type DocumentSourceRef struct {
	Ref       BlobRef `json:"ref" check:"func=Validate"`
	MediaType string  `json:"mediaType"`
	FileName  string  `json:"fileName"`
}

func (DocumentSourceRef) isDocumentSource() {}

//demi:union tag=type
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

//demi:variant gone
type ToolResultContentBlockGone struct {
	Kind      ModelMediaKind `json:"kind"`
	MediaType string         `json:"mediaType"`
	Cause     GoneCause      `json:"cause"`
}

func (ToolResultContentBlockGone) isToolResultContentBlock() {}

//demi:union tag=type
type GoneCause interface{ isGoneCause() }

//demi:variant not_stored
type GoneCauseNotStored struct {
	Error string `json:"error"`
}

func (GoneCauseNotStored) isGoneCause() {}

//demi:variant retired
type GoneCauseRetired struct {
	At Timestamp `json:"at" check:"func=Validate"`
}

func (GoneCauseRetired) isGoneCause() {}

//demi:union tag=type
type ToolMediaSource interface{ isToolMediaSource() }

//demi:variant ref
type ToolMediaSourceRef struct {
	Ref       BlobRef `json:"ref" check:"func=Validate"`
	MediaType string  `json:"mediaType"`
}

func (ToolMediaSourceRef) isToolMediaSource() {}
