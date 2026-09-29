package webapi

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"strconv"
	"strings"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/internal/wire"
	"github.com/wspl/demi/go/webapi/internal/hostpath"
)

// An entry's complete manual model list: 1 to 1000 models with distinct
// ids.
//
//demi:opaque
//wiregen:browser transparent Models
type ConfiguredModels struct {
	Models []ConfiguredModel `json:"models" check:"items=1..1000"`
}

func (v ConfiguredModels) validate() error {
	var failures []error
	if len(v.Models) < 1 || len(v.Models) > 1000 {
		failures = append(failures, &InvalidError{Rule: "must contain 1 to 1000 models"})
	}
	seen := make(map[Trimmed]bool, len(v.Models))
	for i, model := range v.Models {
		if err := model.validate(); err != nil {
			failures = append(failures, wire.In(wire.Index(i), err))
		}
		if seen[model.ID] {
			failures = append(failures, &InvalidError{Path: wire.Index(i) + ".id", Rule: "must be distinct"})
		}
		seen[model.ID] = true
	}
	return errors.Join(failures...)
}
func (v ConfiguredModels) MarshalJSONTo(enc *jsontext.Encoder) error {
	if err := v.validate(); err != nil {
		return err
	}
	return json.MarshalEncode(enc, v.Models)
}
func (v *ConfiguredModels) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	if dec.PeekKind() != '[' {
		return &InvalidError{Rule: "must be an array"}
	}
	var next ConfiguredModels
	if err := json.UnmarshalDecode(dec, &next.Models); err != nil {
		return err
	}
	if err := next.validate(); err != nil {
		return err
	}
	*v = next
	return nil
}
func (v ConfiguredModel) check() error {
	if v.OutputLimit != nil && *v.OutputLimit > v.ContextWindow {
		return &InvalidError{Path: "outputLimit", Rule: "must not exceed contextWindow"}
	}
	return nil
}
func (v Role) Outranks(other Role) bool {
	return v == RoleMaster && (other == RoleAdmin || other == RoleUser) || v == RoleAdmin && other == RoleUser
}
func (v NewRole) Role() Role                    { return Role(v) }
func EmptyWorkPanel() WorkPanel                 { return WorkPanel{Selection: "change", Tabs: []PanelTab{}} }
func EmptyConversationDraft() ConversationDraft { return ConversationDraft{Files: []DraftFile{}} }

// Where an expose's traffic goes on its device: `host:port` exactly as
// given, or a bare port, which means `127.0.0.1:<port>`. The host is any
// name or address the device can resolve, an IPv6 address in brackets; the
// port is 1 to 65535.
//
//demi:opaque string
//wiregen:browser {"type":"string"}
type ExposeAddress struct {
	core.Identity[ExposeAddressKind]
}
type ExposeAddressKind struct{}

func (ExposeAddressKind) CheckIdentity(text string) error {
	_, _, err := splitExpose(text)
	return err
}
func ParseExposeAddress(text string) (ExposeAddress, error) {
	if !strings.Contains(text, ":") {
		port, err := exposePort(text)
		if err != nil {
			return ExposeAddress{}, err
		}
		text = "127.0.0.1:" + strconv.Itoa(int(port))
	}
	id, err := core.ParseIdentity[ExposeAddressKind](text)
	return ExposeAddress{Identity: id}, err
}
func (v ExposeAddress) validate() error { return core.Validate(v.Identity) }
func (v ExposeAddress) Host() string {
	host, _, _ := splitExpose(v.String()) // Only constructors and decoding create a valid address.
	return strings.TrimSuffix(strings.TrimPrefix(host, "["), "]")
}
func (v ExposeAddress) Port() uint16 {
	_, port, _ := splitExpose(v.String()) // The zero value has no port.
	return port
}
func (v *ExposeAddress) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	text, err := wire.ReadString(dec)
	if err != nil {
		return err
	}
	next, err := ParseExposeAddress(text)
	if err != nil {
		return err
	}
	*v = next
	return nil
}

var errExposeAddress = errors.New("must be host:port or a port, the port 1 to 65535")

// splitExpose applies the expose contract's host syntax without resolving it.
func splitExpose(text string) (string, uint16, error) {
	colon := strings.LastIndexByte(text, ':')
	if colon < 1 {
		return "", 0, errExposeAddress
	}
	host := text[:colon]
	for _, c := range host {
		if c < 33 || c > 126 || c == '/' {
			return "", 0, errExposeAddress
		}
	}
	bracketed := strings.HasPrefix(host, "[") || strings.HasSuffix(host, "]")
	if bracketed && !(len(host) > 2 && strings.HasPrefix(host, "[") && strings.HasSuffix(host, "]")) || !bracketed && strings.Contains(host, ":") {
		return "", 0, errExposeAddress
	}
	port, err := exposePort(text[colon+1:])
	return host, port, err
}

// exposePort accepts only the decimal port syntax of an expose address.
func exposePort(text string) (uint16, error) {
	for _, c := range text {
		if c < '0' || c > '9' {
			return 0, errExposeAddress
		}
	}
	port, err := strconv.ParseUint(text, 10, 16)
	if err != nil || port == 0 {
		return 0, errExposeAddress
	}
	return uint16(port), nil
}

// A path on a Host that names its root: `/…` on Unix, `C:\…` on Windows.
// The Host decides what it names, so the meaning never depends on where a
// runner stands.
//
//demi:opaque string
//wiregen:browser {"type":"string"}
type AbsolutePath struct {
	core.Identity[AbsolutePathKind]
}
type AbsolutePathKind struct{}

func (AbsolutePathKind) CheckIdentity(text string) error {
	if !hostpath.Absolute(text) {
		return &InvalidError{Rule: "path must be an absolute path on the Host"}
	}
	return nil
}
func ParseAbsolutePath(text string) (AbsolutePath, error) {
	id, err := core.ParseIdentity[AbsolutePathKind](text)
	return AbsolutePath{Identity: id}, err
}
func (v AbsolutePath) validate() error { return core.Validate(v.Identity) }

type TreePath struct{ core.Identity[TreePathKind] }
type TreePathKind struct{}

func (TreePathKind) CheckIdentity(text string) error {
	if text == "" || strings.HasPrefix(text, "/") {
		return &InvalidError{Rule: "path must be a relative path inside the working tree"}
	}
	for _, part := range strings.Split(text, "/") {
		if part == ".." {
			return &InvalidError{Rule: "path must be a relative path inside the working tree"}
		}
	}
	return nil
}
func ParseTreePath(text string) (TreePath, error) {
	id, err := core.ParseIdentity[TreePathKind](text)
	return TreePath{Identity: id}, err
}

type NonEmptyPath struct {
	core.Identity[core.NonemptyIdentity]
}

func ParseNonEmptyPath(text string) (NonEmptyPath, error) {
	id, err := core.ParseIdentity[core.NonemptyIdentity](text)
	return NonEmptyPath{Identity: id}, err
}
