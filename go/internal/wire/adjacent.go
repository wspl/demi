package wire

import (
	"bytes"
	"encoding/json/jsontext"
	"encoding/json/v2"
)

// ReadAdjacent reads the object of an adjacently tagged union (serde's
// #[serde(tag = "t", content = "c")]): its tag member and its content member,
// each once, and nothing else. The content is returned as JSON, for the variant
// the tag names to decode.
func ReadAdjacent(dec *jsontext.Decoder, tagName, contentName string) (tag string, content jsontext.Value, err error) {
	if err := BeginObject(dec); err != nil {
		return "", nil, err
	}
	var haveTag bool
	for {
		name, more, err := NextMember(dec)
		if err != nil {
			return "", nil, err
		}
		if !more {
			break
		}
		switch name {
		case tagName:
			haveTag = true
			if tag, err = ReadString(dec); err != nil {
				return "", nil, In(name, err)
			}
		case contentName:
			if content, err = ReadRaw(dec); err != nil {
				return "", nil, In(name, err)
			}
		default:
			return "", nil, Unknown(name)
		}
	}
	if !haveTag {
		return "", nil, Required(tagName)
	}
	if content == nil {
		return "", nil, Required(contentName)
	}
	return tag, content, nil
}

// NestIn returns err, the refusal of the value of the member elem, as a refusal
// with elem in front of each of its paths; a nil err stays nil.
func NestIn(elem string, err error) error {
	var report Report
	report.Nest(elem, err)
	return report.Err()
}

// WriteMembers encodes value, which encodes as a JSON object, and writes its
// members into the object enc is writing, as the members of that object: the
// encoding of a struct with an inline union.
func WriteMembers(enc *jsontext.Encoder, value any, opts ...json.Options) error {
	encoded, err := json.Marshal(value, opts...)
	if err != nil {
		return err
	}
	dec := jsontext.NewDecoder(bytes.NewReader(encoded))
	if err := BeginObject(dec); err != nil {
		return err
	}
	for {
		name, more, err := NextMember(dec)
		if err != nil {
			return err
		}
		if !more {
			return nil
		}
		if err := enc.WriteToken(jsontext.String(name)); err != nil {
			return err
		}
		member, err := dec.ReadValue()
		if err != nil {
			return err
		}
		if err := enc.WriteValue(member); err != nil {
			return err
		}
	}
}
