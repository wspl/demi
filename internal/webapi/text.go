package webapi

// An email address as the product keeps it: trimmed and lowercased, at most
// 254 characters, and of the form the web app's schema accepts. The
// type cannot hold another value, so storage lookups and uniqueness see one
// spelling of each address.
// +demi:id
// +demi:schema-primitive
// +demi:format email
// +demi:length chars max=254
type EmailAddress string

// Text whose surrounding white space is removed when it arrives, as a name
// a user types; the field's garde rule bounds what remains. Its schema's
// `trimmed` format tells the web app's schema to trim before it checks the
// bounds.
// +demi:id
// +demi:schema-primitive
// +demi:format trimmed
type Trimmed string

// The base URL of a vendor's API, as an entry configures it: an `http` or
// `https` URL (`providers.md` § Endpoints), which its schema's `http-url`
// format tells the web app's schema.
// +demi:id
// +demi:schema-primitive
// +demi:format http-url
type EndpointURL string

// The most characters (Unicode scalar values) an email address has.
const EmailMax = 254
