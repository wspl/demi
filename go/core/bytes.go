package core

import (
	"encoding/base64"
	"encoding/json/jsontext"
	"fmt"
	"strings"

	"github.com/wspl/demi/go/internal/wire"
)

// B64Bytes owns bytes encoded as padded, standard RFC 4648 base64.
//
//demi:opaque string
//wiregen:browser inline {"type":"string"}
type B64Bytes struct{ data string }

func NewB64Bytes(data []byte) B64Bytes { return B64Bytes{data: string(data)} }
func (b B64Bytes) Bytes() []byte       { return []byte(b.data) }
func (b B64Bytes) Len() int            { return len(b.data) }
func (b B64Bytes) Base64Len() uint64   { return uint64(base64.StdEncoding.EncodedLen(len(b.data))) }
func (b B64Bytes) String() string      { return fmt.Sprintf("B64Bytes(%d bytes)", len(b.data)) }
func (b B64Bytes) validate() error     { return nil }
func (b B64Bytes) MarshalJSONTo(enc *jsontext.Encoder) error {
	return enc.WriteToken(jsontext.String(base64.StdEncoding.EncodeToString([]byte(b.data))))
}
func (b *B64Bytes) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	text, err := wire.ReadString(dec)
	if err != nil {
		return err
	}
	// Go's base64 decoder ignores CR and LF; Rust's STANDARD rejects both.
	if strings.ContainsAny(text, "\r\n") {
		return &InvalidError{Rule: "invalid base64"}
	}
	data, err := base64.StdEncoding.Strict().DecodeString(text)
	if err != nil {
		return &InvalidError{Rule: "invalid base64"}
	}
	*b = NewB64Bytes(data)
	return nil
}
