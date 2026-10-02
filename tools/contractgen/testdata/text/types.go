package text

//go:generate go run ../..

// +demi:root direction=send output=web
// +demi:format email
// +demi:msgpack
// +demi:id
type Email string

// +demi:root direction=send output=web
// +demi:format trimmed
// +demi:length chars min=1 max=8
// +demi:msgpack
type Name string

// +demi:root direction=receive output=web
// +demi:format email
type ReceivedEmail string

// +demi:root direction=receive output=web
type Received struct {
	Email Email `json:"email"`
}

// +demi:root direction=send output=web
// +demi:format http-url
// +demi:msgpack
// +demi:id
type EndpointURL string
