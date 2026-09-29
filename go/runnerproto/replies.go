package runnerproto

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"github.com/tinylib/msgp/msgp"
	"github.com/wspl/demi/go/internal/wire"
	"unicode/utf8"
)

// replyFields reads the envelope in order because op determines result's type.
func replyFields(data []byte, packed bool, tag string) (string, string, []byte, error) {
	var fields []wire.MPMember
	var err error
	if packed {
		fields, err = wire.MPObject(data)
	} else {
		dec := wire.NewDecoder(data)
		if _, err = wire.ReadObject(dec); err != nil {
			return "", "", nil, err
		}
		dec = wire.NewDecoder(data)
		_, err = dec.ReadToken()
		for err == nil && dec.PeekKind() != '}' {
			var key jsontext.Token
			key, err = dec.ReadToken()
			if err != nil {
				break
			}
			name := key.String()
			var value jsontext.Value
			value, err = dec.ReadValue()
			fields = append(fields, wire.MPMember{Name: name, Data: append([]byte(nil), value...)})
		}
	}
	if err != nil {
		return "", "", nil, err
	}
	seen := map[string]bool{}
	var id, op string
	var result []byte
	for _, field := range fields {
		if seen[field.Name] {
			return "", "", nil, &InvalidError{Path: field.Name, Rule: "duplicate field"}
		}
		seen[field.Name] = true
		readString := func() (string, error) {
			if packed {
				return wire.MPString(field.Data)
			}
			return wire.ReadString(wire.NewDecoder(field.Data))
		}
		switch field.Name {
		case "type":
			var value string
			value, err = readString()
			if err == nil && value != tag {
				err = &InvalidError{Rule: "wrong reply type"}
			}
		case "id":
			id, err = readString()
		case "op":
			op, err = readString()
		case "result":
			if !seen["op"] {
				return "", "", nil, &InvalidError{Path: "result", Rule: "op must precede result"}
			}
			result = field.Data
		default:
			return "", "", nil, wire.Unknown(field.Name)
		}
		if err != nil {
			return "", "", nil, wire.In(field.Name, err)
		}
	}
	for _, key := range []string{"type", "id", "op", "result"} {
		if !seen[key] {
			return "", "", nil, wire.Required(key)
		}
	}
	return id, op, result, nil
}

// encodeReply preserves the protocol's type, id, op, result order.
func encodeReply(tag, id, op string, result []byte, packed bool) ([]byte, error) {
	if !utf8.ValidString(id) {
		return nil, &InvalidError{Path: "id", Rule: "invalid UTF-8"}
	}
	if packed {
		data := msgp.AppendMapHeader(nil, 4)
		for _, pair := range [][2]string{{"type", tag}, {"id", id}, {"op", op}} {
			data = msgp.AppendString(data, pair[0])
			data = msgp.AppendString(data, pair[1])
		}
		data = msgp.AppendString(data, "result")
		return append(data, result...), nil
	}
	data := []byte(`{"type":`)
	for i, value := range []string{tag, id, op} {
		if i == 1 {
			data = append(data, `,"id":`...)
		}
		if i == 2 {
			data = append(data, `,"op":`...)
		}
		encoded, err := json.Marshal(value)
		if err != nil {
			return nil, wire.Refusal(err)
		}
		data = append(data, encoded...)
	}
	data = append(data, `,"result":`...)
	data = append(data, result...)
	return append(data, '}'), nil
}

// FSResult is the closed set of successful fs operation results.
type FSResult interface{ fsResult() }
type FSResultReadFile struct{}

func (FSResultReadFile) fsResult() {}

type FSResultWriteFile struct{}

func (FSResultWriteFile) fsResult() {}

type FSResultExists struct{ Value bool }

func (FSResultExists) fsResult() {}

type FSResultStat struct{ Value FileStat }

func (FSResultStat) fsResult() {}

type FSResultLstat struct{ Value FileStat }

func (FSResultLstat) fsResult() {}

type FSResultReaddir struct{ Value []DirEntry }

func (FSResultReaddir) fsResult() {}

type FSResultMkdir struct{}

func (FSResultMkdir) fsResult() {}

type FSResultRm struct{}

func (FSResultRm) fsResult() {}

type FSResultCp struct{}

func (FSResultCp) fsResult() {}

type FSResultMv struct{}

func (FSResultMv) fsResult() {}

type FSResultChmod struct{}

func (FSResultChmod) fsResult() {}

type FSResultSymlink struct{}

func (FSResultSymlink) fsResult() {}

type FSResultLink struct{}

func (FSResultLink) fsResult() {}

type FSResultReadlink struct{ Value string }

func (FSResultReadlink) fsResult() {}

type FSResultRealpath struct{ Value string }

func (FSResultRealpath) fsResult() {}

type FSResultUtimes struct{}

func (FSResultUtimes) fsResult() {}

// OutboundFSOk carries the operation before its typed result, as replies.rs requires.
//
//demi:variant fs_ok opaque
type OutboundFSOk struct {
	ID     string
	Result FSResult
}

func (OutboundFSOk) outbound() {}
func (v OutboundFSOk) validate() error {
	if !utf8.ValidString(v.ID) {
		return &InvalidError{Path: "id", Rule: "invalid UTF-8"}
	}
	err := func() error {
		switch value := v.Result.(type) {
		case FSResultReadFile:
			return nil
		case FSResultWriteFile:
			return nil
		case FSResultExists:
			return nil
		case FSResultStat:
			return value.Value.validate()
		case FSResultLstat:
			return value.Value.validate()
		case FSResultReaddir:
			for i, item := range value.Value {
				if err := item.validate(); err != nil {
					return wire.In(wire.Index(i), err)
				}
			}
			return nil
		case FSResultMkdir:
			return nil
		case FSResultRm:
			return nil
		case FSResultCp:
			return nil
		case FSResultMv:
			return nil
		case FSResultChmod:
			return nil
		case FSResultSymlink:
			return nil
		case FSResultLink:
			return nil
		case FSResultReadlink:
			if !utf8.ValidString(value.Value) {
				return &InvalidError{Rule: "invalid UTF-8"}
			}
			return nil
		case FSResultRealpath:
			if !utf8.ValidString(value.Value) {
				return &InvalidError{Rule: "invalid UTF-8"}
			}
			return nil
		case FSResultUtimes:
			return nil
		}
		return &InvalidError{Rule: "unknown result variant"}
	}()
	if err != nil {
		return wire.In("result", err)
	}
	return nil
}
func (v *OutboundFSOk) read(data []byte, packed bool) error {
	id, op, result, err := replyFields(data, packed, "fs_ok")
	if err != nil {
		return err
	}
	decoded, err := decodeFSResult(op, result, packed)
	if err != nil {
		var invalid *InvalidError
		if errors.As(err, &invalid) && invalid.Path == "op" {
			return err
		}
		return wire.In("result", err)
	}
	*v = OutboundFSOk{ID: id, Result: decoded}
	return nil
}
func (v *OutboundFSOk) UnmarshalMsgpack(data []byte) error { return v.read(data, true) }
func (v *OutboundFSOk) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	data, err := dec.ReadValue()
	if err != nil {
		return err
	}
	return v.read(data, false)
}
func (v OutboundFSOk) MarshalMsgpack() ([]byte, error) {
	op, result, err := encodeFSResult(v.Result, true)
	if err != nil {
		return nil, wire.In("result", err)
	}
	return encodeReply("fs_ok", v.ID, op, result, true)
}
func (v OutboundFSOk) MarshalJSONTo(enc *jsontext.Encoder) error {
	op, result, err := encodeFSResult(v.Result, false)
	if err != nil {
		return wire.In("result", err)
	}
	data, err := encodeReply("fs_ok", v.ID, op, result, false)
	if err != nil {
		return err
	}
	return enc.WriteValue(data)
}
func decodeFSResult(op string, data []byte, packed bool) (FSResult, error) {
	switch op {
	case "readFile":
		if err := emptyReplyResult(data, packed); err != nil {
			return nil, err
		}
		return FSResultReadFile{}, nil
	case "writeFile":
		if err := emptyReplyResult(data, packed); err != nil {
			return nil, err
		}
		return FSResultWriteFile{}, nil
	case "exists":
		var v bool
		var err error
		if packed {
			v, err = wire.MPRead(data, msgp.ReadBoolBytes)
		} else {
			v, err = wire.ReadBool(wire.NewDecoder(data))
		}
		if err != nil {
			return nil, wire.Refusal(err)
		}
		return FSResultExists{Value: v}, nil
	case "stat":
		var v FileStat
		var err error
		if packed {
			v, err = DecodeFileStatMsgpack(data)
		} else {
			err = json.Unmarshal(data, &v)
		}
		if err != nil {
			return nil, wire.Refusal(err)
		}
		return FSResultStat{Value: v}, nil
	case "lstat":
		var v FileStat
		var err error
		if packed {
			v, err = DecodeFileStatMsgpack(data)
		} else {
			err = json.Unmarshal(data, &v)
		}
		if err != nil {
			return nil, wire.Refusal(err)
		}
		return FSResultLstat{Value: v}, nil
	case "readdir":
		var v []DirEntry
		var err error
		if packed {
			v, err = wire.MPArray(data, DecodeDirEntryMsgpack)
		} else {
			err = wire.BeginArray(wire.NewDecoder(data))
			if err == nil {
				err = json.Unmarshal(data, &v)
			}
		}
		if err != nil {
			return nil, wire.Refusal(err)
		}
		return FSResultReaddir{Value: v}, nil
	case "mkdir":
		if err := emptyReplyResult(data, packed); err != nil {
			return nil, err
		}
		return FSResultMkdir{}, nil
	case "rm":
		if err := emptyReplyResult(data, packed); err != nil {
			return nil, err
		}
		return FSResultRm{}, nil
	case "cp":
		if err := emptyReplyResult(data, packed); err != nil {
			return nil, err
		}
		return FSResultCp{}, nil
	case "mv":
		if err := emptyReplyResult(data, packed); err != nil {
			return nil, err
		}
		return FSResultMv{}, nil
	case "chmod":
		if err := emptyReplyResult(data, packed); err != nil {
			return nil, err
		}
		return FSResultChmod{}, nil
	case "symlink":
		if err := emptyReplyResult(data, packed); err != nil {
			return nil, err
		}
		return FSResultSymlink{}, nil
	case "link":
		if err := emptyReplyResult(data, packed); err != nil {
			return nil, err
		}
		return FSResultLink{}, nil
	case "readlink":
		var v string
		var err error
		if packed {
			v, err = wire.MPString(data)
		} else {
			v, err = wire.ReadString(wire.NewDecoder(data))
		}
		if err != nil {
			return nil, wire.Refusal(err)
		}
		return FSResultReadlink{Value: v}, nil
	case "realpath":
		var v string
		var err error
		if packed {
			v, err = wire.MPString(data)
		} else {
			v, err = wire.ReadString(wire.NewDecoder(data))
		}
		if err != nil {
			return nil, wire.Refusal(err)
		}
		return FSResultRealpath{Value: v}, nil
	case "utimes":
		if err := emptyReplyResult(data, packed); err != nil {
			return nil, err
		}
		return FSResultUtimes{}, nil
	}
	return nil, &InvalidError{Path: "op", Rule: "unknown operation"}
}
func encodeFSResult(value FSResult, packed bool) (string, []byte, error) {
	switch v := value.(type) {
	case FSResultReadFile:
		if packed {
			return "readFile", msgp.AppendNil(nil), nil
		}
		return "readFile", []byte("null"), nil
	case FSResultWriteFile:
		if packed {
			return "writeFile", msgp.AppendNil(nil), nil
		}
		return "writeFile", []byte("null"), nil
	case FSResultExists:
		if !packed {
			data, err := json.Marshal(v.Value, json.Deterministic(true))
			if err != nil {
				return "", nil, wire.Refusal(err)
			}
			return "exists", data, nil
		}
		return "exists", msgp.AppendBool(nil, v.Value), nil
	case FSResultStat:
		if !packed {
			data, err := json.Marshal(v.Value, json.Deterministic(true))
			if err != nil {
				return "", nil, wire.Refusal(err)
			}
			return "stat", data, nil
		}
		data, err := v.Value.MarshalMsgpack()
		return "stat", data, err
	case FSResultLstat:
		if !packed {
			data, err := json.Marshal(v.Value, json.Deterministic(true))
			if err != nil {
				return "", nil, wire.Refusal(err)
			}
			return "lstat", data, nil
		}
		data, err := v.Value.MarshalMsgpack()
		return "lstat", data, err
	case FSResultReaddir:
		if !packed {
			data, err := json.Marshal(v.Value, json.Deterministic(true))
			if err != nil {
				return "", nil, wire.Refusal(err)
			}
			return "readdir", data, nil
		}
		data := msgp.AppendArrayHeader(nil, uint32(len(v.Value)))
		for i, item := range v.Value {
			encoded, err := item.MarshalMsgpack()
			if err != nil {
				return "", nil, wire.In(wire.Index(i), err)
			}
			data = append(data, encoded...)
		}
		return "readdir", data, nil
	case FSResultMkdir:
		if packed {
			return "mkdir", msgp.AppendNil(nil), nil
		}
		return "mkdir", []byte("null"), nil
	case FSResultRm:
		if packed {
			return "rm", msgp.AppendNil(nil), nil
		}
		return "rm", []byte("null"), nil
	case FSResultCp:
		if packed {
			return "cp", msgp.AppendNil(nil), nil
		}
		return "cp", []byte("null"), nil
	case FSResultMv:
		if packed {
			return "mv", msgp.AppendNil(nil), nil
		}
		return "mv", []byte("null"), nil
	case FSResultChmod:
		if packed {
			return "chmod", msgp.AppendNil(nil), nil
		}
		return "chmod", []byte("null"), nil
	case FSResultSymlink:
		if packed {
			return "symlink", msgp.AppendNil(nil), nil
		}
		return "symlink", []byte("null"), nil
	case FSResultLink:
		if packed {
			return "link", msgp.AppendNil(nil), nil
		}
		return "link", []byte("null"), nil
	case FSResultReadlink:
		if !utf8.ValidString(v.Value) {
			return "", nil, &InvalidError{Rule: "invalid UTF-8"}
		}
		if !packed {
			data, err := json.Marshal(v.Value, json.Deterministic(true))
			if err != nil {
				return "", nil, wire.Refusal(err)
			}
			return "readlink", data, nil
		}
		return "readlink", msgp.AppendString(nil, v.Value), nil
	case FSResultRealpath:
		if !utf8.ValidString(v.Value) {
			return "", nil, &InvalidError{Rule: "invalid UTF-8"}
		}
		if !packed {
			data, err := json.Marshal(v.Value, json.Deterministic(true))
			if err != nil {
				return "", nil, wire.Refusal(err)
			}
			return "realpath", data, nil
		}
		return "realpath", msgp.AppendString(nil, v.Value), nil
	case FSResultUtimes:
		if packed {
			return "utimes", msgp.AppendNil(nil), nil
		}
		return "utimes", []byte("null"), nil
	}
	return "", nil, &InvalidError{Rule: "unknown result variant"}
}

// GitResult is the closed set of successful git operation results.
type GitResult interface{ gitResult() }
type GitResultChanges struct{ Value GitChanges }

func (GitResultChanges) gitResult() {}

type GitResultShow struct{}

func (GitResultShow) gitResult() {}

// OutboundGitOk carries the operation before its typed result, as replies.rs requires.
//
//demi:variant git_ok opaque
type OutboundGitOk struct {
	ID     string
	Result GitResult
}

func (OutboundGitOk) outbound() {}
func (v OutboundGitOk) validate() error {
	if !utf8.ValidString(v.ID) {
		return &InvalidError{Path: "id", Rule: "invalid UTF-8"}
	}
	err := func() error {
		switch value := v.Result.(type) {
		case GitResultChanges:
			return value.Value.validate()
		case GitResultShow:
			return nil
		}
		return &InvalidError{Rule: "unknown result variant"}
	}()
	if err != nil {
		return wire.In("result", err)
	}
	return nil
}
func (v *OutboundGitOk) read(data []byte, packed bool) error {
	id, op, result, err := replyFields(data, packed, "git_ok")
	if err != nil {
		return err
	}
	decoded, err := decodeGitResult(op, result, packed)
	if err != nil {
		var invalid *InvalidError
		if errors.As(err, &invalid) && invalid.Path == "op" {
			return err
		}
		return wire.In("result", err)
	}
	*v = OutboundGitOk{ID: id, Result: decoded}
	return nil
}
func (v *OutboundGitOk) UnmarshalMsgpack(data []byte) error { return v.read(data, true) }
func (v *OutboundGitOk) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	data, err := dec.ReadValue()
	if err != nil {
		return err
	}
	return v.read(data, false)
}
func (v OutboundGitOk) MarshalMsgpack() ([]byte, error) {
	op, result, err := encodeGitResult(v.Result, true)
	if err != nil {
		return nil, wire.In("result", err)
	}
	return encodeReply("git_ok", v.ID, op, result, true)
}
func (v OutboundGitOk) MarshalJSONTo(enc *jsontext.Encoder) error {
	op, result, err := encodeGitResult(v.Result, false)
	if err != nil {
		return wire.In("result", err)
	}
	data, err := encodeReply("git_ok", v.ID, op, result, false)
	if err != nil {
		return err
	}
	return enc.WriteValue(data)
}
func decodeGitResult(op string, data []byte, packed bool) (GitResult, error) {
	switch op {
	case "changes":
		var v GitChanges
		var err error
		if packed {
			v, err = DecodeGitChangesMsgpack(data)
		} else {
			err = json.Unmarshal(data, &v)
		}
		if err != nil {
			return nil, wire.Refusal(err)
		}
		if err = v.validate(); err != nil {
			return nil, err
		}
		return GitResultChanges{Value: v}, nil
	case "show":
		if err := emptyReplyResult(data, packed); err != nil {
			return nil, err
		}
		return GitResultShow{}, nil
	}
	return nil, &InvalidError{Path: "op", Rule: "unknown operation"}
}
func encodeGitResult(value GitResult, packed bool) (string, []byte, error) {
	switch v := value.(type) {
	case GitResultChanges:
		if err := v.Value.validate(); err != nil {
			return "", nil, err
		}
		if !packed {
			data, err := json.Marshal(v.Value, json.Deterministic(true))
			if err != nil {
				return "", nil, wire.Refusal(err)
			}
			return "changes", data, nil
		}
		data, err := v.Value.MarshalMsgpack()
		return "changes", data, err
	case GitResultShow:
		if packed {
			return "show", msgp.AppendNil(nil), nil
		}
		return "show", []byte("null"), nil
	}
	return "", nil, &InvalidError{Rule: "unknown result variant"}
}

// emptyReplyResult requires nil for an operation that carries no result.
func emptyReplyResult(data []byte, packed bool) error {
	if packed {
		if len(data) == 1 && msgp.IsNil(data) {
			return nil
		}
	} else if string(data) == "null" {
		return nil
	}
	return &InvalidError{Rule: "expected nil"}
}
