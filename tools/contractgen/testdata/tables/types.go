package tables

type PreviewType struct {
	MediaType  string   `json:"mediaType"`
	Extensions []string `json:"extensions"`
	InPlace    bool     `json:"inPlace"`
}

// +demi:table
var PREVIEW_TYPES = []PreviewType{
	{MediaType: "image/png", Extensions: []string{"png"}, InPlace: true},
	{MediaType: "image/jpeg", Extensions: []string{"jpg", "jpeg"}, InPlace: true},
	{MediaType: "image/gif", Extensions: []string{"gif"}, InPlace: true},
	{MediaType: "image/webp", Extensions: []string{"webp"}, InPlace: true},
	{MediaType: "image/avif", Extensions: []string{"avif"}, InPlace: true},
	{MediaType: "image/bmp", Extensions: []string{"bmp"}, InPlace: true},
	{MediaType: "image/x-icon", Extensions: []string{"ico"}, InPlace: true},
	{MediaType: "image/svg+xml", Extensions: []string{"svg"}, InPlace: true},
	{MediaType: "video/mp4", Extensions: []string{"mp4"}, InPlace: true},
	{MediaType: "video/x-m4v", Extensions: []string{"m4v"}, InPlace: true},
	{MediaType: "video/webm", Extensions: []string{"webm"}, InPlace: true},
	{MediaType: "video/quicktime", Extensions: []string{"mov"}, InPlace: true},
	{MediaType: "audio/mpeg", Extensions: []string{"mp3"}, InPlace: true},
	{MediaType: "audio/wav", Extensions: []string{"wav"}, InPlace: true},
	{MediaType: "audio/ogg", Extensions: []string{"ogg", "oga", "opus"}, InPlace: true},
	{MediaType: "audio/mp4", Extensions: []string{"m4a"}, InPlace: true},
	{MediaType: "audio/aac", Extensions: []string{"aac"}, InPlace: true},
	{MediaType: "audio/flac", Extensions: []string{"flac"}, InPlace: true},
	{MediaType: "audio/webm", Extensions: []string{"weba"}, InPlace: true},
	{MediaType: "application/pdf", Extensions: []string{"pdf"}, InPlace: true},
	{MediaType: "text/markdown", Extensions: []string{"md", "markdown"}, InPlace: false},
}

// ModelFileType records whether a model reads an extension as an attachment or video.
type ModelFileType struct {
	Extension string `json:"extension"`
	Kind      string `json:"kind"`
}

// +demi:table
var ModelFileTypes = []ModelFileType{
	{"png", "attachment"}, {"jpg", "attachment"}, {"jpeg", "attachment"},
	{"gif", "attachment"}, {"webp", "attachment"}, {"pdf", "attachment"},
	{"mp4", "video"}, {"mov", "video"}, {"webm", "video"}, {"m4v", "video"},
}

type FrameConstant struct {
	Name  string `json:"name"`
	Value uint32 `json:"value"`
}

// +demi:table
var LiveViewFrameConstants = []FrameConstant{
	{"CONTROL_FRAME", 1}, {"VIDEO_FRAME", 2}, {"FILE_FRAME", 3},
	{"MAX_FRAME_BYTES", 16 * 1024 * 1024}, {"FILE_CHUNK_BYTES", 64 * 1024},
	{"HEARTBEAT_MS", 250}, {"STALL_MS", 1000},
}
