package browserop

// What the page sends.
// +demi:schema
// +demi:union tag=type
//
// +demi:root direction=send output=plugin-browser
//
//sumtype:decl
type LiveViewerMessage interface{ liveViewerMessage() }

// First: the viewer's platform.
// +demi:variant hello
type LiveViewerMessageHello struct {
	Platform Platform `json:"platform"`
}

func (*LiveViewerMessageHello) liveViewerMessage() {}

// The panel's size in CSS pixels and the viewer's screen.
// +demi:variant panel
type LiveViewerMessagePanel struct {
	// +demi:range min=1 max=4096
	Width uint32 `json:"width"`
	// +demi:range min=1 max=4096
	Height uint32 `json:"height"`
	// +demi:range min=0.5 max=4.0
	DevicePixelRatio float64 `json:"devicePixelRatio"`
	// +demi:range min=1 max=4096
	ScreenWidth uint32 `json:"screenWidth"`
	// +demi:range min=1 max=4096
	ScreenHeight uint32 `json:"screenHeight"`
}

func (*LiveViewerMessagePanel) liveViewerMessage() {}

// The tab the view shows, or none.
// +demi:variant watch
type LiveViewerMessageWatch struct {
	// +demi:nullable
	Tab *TabID `json:"tab"`
}

func (*LiveViewerMessageWatch) liveViewerMessage() {}

// +demi:variant mode
type LiveViewerMessageMode struct {
	Tab  TabID      `json:"tab"`
	Mode ViewerMode `json:"mode"`
}

func (*LiveViewerMessageMode) liveViewerMessage() {}

// +demi:variant pointer
type LiveViewerMessagePointer struct {
	Tab    TabID         `json:"tab"`
	Action PointerAction `json:"action"`
	// +demi:range min=0.0 max=4096.0
	X float64 `json:"x"`
	// +demi:range min=0.0 max=4096.0
	Y      float64       `json:"y"`
	Button PointerButton `json:"button"`
	// +demi:range max=31
	Buttons uint8 `json:"buttons"`
	// +demi:range max=3
	ClickCount uint8 `json:"clickCount"`
	// The modifier keys held, as CDP numbers them: Alt 1, Control 2,
	// Meta 4, Shift 8.
	// +demi:range max=15
	Modifiers uint8 `json:"modifiers"`
}

func (*LiveViewerMessagePointer) liveViewerMessage() {}

// +demi:variant wheel
type LiveViewerMessageWheel struct {
	Tab TabID `json:"tab"`
	// +demi:range min=0.0 max=4096.0
	X float64 `json:"x"`
	// +demi:range min=0.0 max=4096.0
	Y float64 `json:"y"`
	// +demi:range min=-10000.0 max=10000.0
	DeltaX float64 `json:"deltaX"`
	// +demi:range min=-10000.0 max=10000.0
	DeltaY float64 `json:"deltaY"`
	// +demi:range max=15
	Modifiers uint8 `json:"modifiers"`
}

func (*LiveViewerMessageWheel) liveViewerMessage() {}

// +demi:variant key
type LiveViewerMessageKey struct {
	Tab    TabID     `json:"tab"`
	Action KeyAction `json:"action"`
	// +demi:length chars max=64
	Key string `json:"key"`
	// +demi:length chars max=64
	Code    string `json:"code"`
	KeyCode uint8  `json:"keyCode"`
	// +demi:range max=15
	Modifiers uint8 `json:"modifiers"`
	Repeat    bool  `json:"repeat"`
	// +demi:range max=3
	Location uint8 `json:"location"`
	// +demi:length chars min=1 max=16
	Text     *string `json:"text,omitempty"`
	AltGraph bool    `json:"altGraph"`
}

func (*LiveViewerMessageKey) liveViewerMessage() {}

// Committed text: an input method's result.
// +demi:variant text
type LiveViewerMessageText struct {
	Tab TabID `json:"tab"`
	// +demi:length chars max=20000
	Text string `json:"text"`
}

func (*LiveViewerMessageText) liveViewerMessage() {}

// An input method's text being composed.
// +demi:variant composition
type LiveViewerMessageComposition struct {
	Tab TabID `json:"tab"`
	// +demi:length chars max=20000
	Text string `json:"text"`
}

func (*LiveViewerMessageComposition) liveViewerMessage() {}

// +demi:variant paste
type LiveViewerMessagePaste struct {
	Tab TabID `json:"tab"`
	// +demi:length chars max=1000000
	Text string `json:"text"`
	// +demi:length chars max=4000000
	HTML string `json:"html"`
}

func (*LiveViewerMessagePaste) liveViewerMessage() {}

// A choice in a native control, for the revision the viewer saw.
// +demi:variant choice
type LiveViewerMessageChoice struct {
	Tab   TabID        `json:"tab"`
	Token ControlToken `json:"token"`
	// +demi:range max=9007199254740991
	Revision uint64 `json:"revision"`
	// +demi:length chars max=10000
	Value string `json:"value"`
	// +demi:length max=1000
	Indices []uint32 `json:"indices"`
}

func (*LiveViewerMessageChoice) liveViewerMessage() {}

// Files chosen for a file input; their bytes follow as file frames.
// +demi:variant upload
type LiveViewerMessageUpload struct {
	Tab   TabID        `json:"tab"`
	Token ControlToken `json:"token"`
	// +demi:range max=9007199254740991
	Revision uint64 `json:"revision"`
	Upload   uint32 `json:"upload"`
	// +demi:length max=100
	Files []UploadFile `json:"files"`
}

func (*LiveViewerMessageUpload) liveViewerMessage() {}

// +demi:variant dialog
type LiveViewerMessageDialog struct {
	Tab    TabID `json:"tab"`
	Accept bool  `json:"accept"`
	// +demi:length chars max=2000
	Text *string `json:"text,omitempty"`
}

func (*LiveViewerMessageDialog) liveViewerMessage() {}

// The page showed this frame; the module paces itself by these.
// +demi:variant ack
type LiveViewerMessageAck struct {
	Generation  uint32 `json:"generation"`
	Sequence    uint32 `json:"sequence"`
	DecodeQueue uint32 `json:"decodeQueue"`
}

func (*LiveViewerMessageAck) liveViewerMessage() {}

// The decoder lost the stream; the next frame must be a key frame.
// +demi:variant keyframe
type LiveViewerMessageKeyframe struct {
	Generation uint32 `json:"generation"`
}

func (*LiveViewerMessageKeyframe) liveViewerMessage() {}

// Release every key and button this viewer holds.
// +demi:variant release
type LiveViewerMessageRelease struct {
}

func (*LiveViewerMessageRelease) liveViewerMessage() {}
