package storage

// Temporary declaration bridge: wiregen cannot load webapi while its
// foreign embedded structs still require webapi/wiredecl. Remove these codecs
// when L5 supplies that feature; the declarations remain beside their types.

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"fmt"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

// storedObject checks required storage members before decoding their values.
func storedObject(dec *jsontext.Decoder, required []string, open bool, nullable ...string) (map[string]jsontext.Value, error) {
	bytes, err := dec.ReadValue()
	if err != nil {
		return nil, err
	}
	var fields map[string]jsontext.Value
	if err = json.Unmarshal(bytes, &fields); err != nil {
		return nil, err
	}
	if fields == nil {
		return nil, fmt.Errorf("expected an object")
	}
	for _, name := range required {
		if _, ok := fields[name]; !ok {
			return nil, fmt.Errorf("missing field `%s`", name)
		}
	}
	for _, name := range required {
		if string(fields[name]) != "null" {
			continue
		}
		allowed := false
		for _, optional := range nullable {
			if name == optional {
				allowed = true
			}
		}
		if !allowed {
			return nil, fmt.Errorf("%s: unexpected null", name)
		}
	}
	if !open {
		for name := range fields {
			found := false
			for _, allowed := range required {
				if name == allowed {
					found = true
					break
				}
			}
			if !found {
				return nil, fmt.Errorf("unknown field `%s`", name)
			}
		}
	}
	return fields, nil
}
func (v *DraftVersion) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	fields, err := storedObject(dec, []string{"text", "files"}, false)
	if err != nil {
		return err
	}
	if err = json.Unmarshal(fields["text"], &v.Text); err != nil {
		return err
	}
	var files []jsontext.Value
	if err = json.Unmarshal(fields["files"], &files); err != nil {
		return err
	}
	if files == nil {
		return fmt.Errorf("files: expected an array")
	}
	v.Files = make([]webapi.DraftFile, len(files))
	for i, bytes := range files {
		value, err := webapi.DecodeDraftFileJSON(bytes)
		if err != nil {
			return err
		}
		v.Files[i] = value
	}
	return nil
}
func decodeExecutionTarget(data jsontext.Value) (ExecutionTarget, error) {
	var kind struct {
		Kind string `json:"kind"`
	}
	if err := json.Unmarshal(data, &kind); err != nil {
		return nil, err
	}
	// Rust's internally tagged variants permit unknown fields, but still require
	// their payload members. Cloud's deviceId is optional and null-as-absent.
	var fields map[string]jsontext.Value
	if err := json.Unmarshal(data, &fields); err != nil {
		return nil, err
	}
	required := []string{"path"}
	switch kind.Kind {
	case "cloud":
	case "device":
		required = append(required, "deviceId")
	case "workspace":
		required = append(required, "deviceId", "workspaceId")
	default:
		return nil, fmt.Errorf("unknown variant `%s`", kind.Kind)
	}
	for _, name := range required {
		if string(fields[name]) == "null" {
			return nil, fmt.Errorf("%s: unexpected null", name)
		}
		if _, ok := fields[name]; !ok {
			return nil, fmt.Errorf("missing field `%s`", name)
		}
	}
	switch kind.Kind {
	case "cloud":
		var v ExecutionTargetCloud
		err := json.Unmarshal(data, &v)
		return v, err
	case "device":
		var v ExecutionTargetDevice
		err := json.Unmarshal(data, &v)
		return v, err
	default:
		var v ExecutionTargetWorkspace
		err := json.Unmarshal(data, &v)
		return v, err
	}
}
func (v *TargetSwitch) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	fields, err := storedObject(dec, []string{"from", "to"}, true)
	if err != nil {
		return err
	}
	v.From, err = decodeExecutionTarget(fields["from"])
	if err != nil {
		return err
	}
	v.To, err = decodeExecutionTarget(fields["to"])
	return err
}
func (v ExecutionTargetCloud) MarshalJSONTo(enc *jsontext.Encoder) error {
	type payload ExecutionTargetCloud
	return json.MarshalEncode(enc, struct {
		Kind string `json:"kind"`
		payload
	}{"cloud", payload(v)})
}
func (v ExecutionTargetDevice) MarshalJSONTo(enc *jsontext.Encoder) error {
	type payload ExecutionTargetDevice
	return json.MarshalEncode(enc, struct {
		Kind string `json:"kind"`
		payload
	}{"device", payload(v)})
}
func (v ExecutionTargetWorkspace) MarshalJSONTo(enc *jsontext.Encoder) error {
	type payload ExecutionTargetWorkspace
	return json.MarshalEncode(enc, struct {
		Kind string `json:"kind"`
		payload
	}{"workspace", payload(v)})
}

func (v *AttachedHostRecord) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	fields, err := storedObject(dec, []string{"device", "name", "cwd"}, false, "cwd")
	if err != nil {
		return err
	}
	if err = json.Unmarshal(fields["device"], &v.Device); err != nil {
		return err
	}
	if err = json.Unmarshal(fields["name"], &v.Name); err != nil {
		return err
	}
	if v.Name == "" {
		return fmt.Errorf("name: length is lower than 1")
	}
	return json.Unmarshal(fields["cwd"], &v.Cwd)
}
func (v *ForkMetadata) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	fields, err := storedObject(dec, []string{"title", "target", "model", "createdAt", "attachedHosts"}, false, "model")
	if err != nil {
		return err
	}
	if err = json.Unmarshal(fields["title"], &v.Title); err != nil {
		return err
	}
	if v.Title == "" {
		return fmt.Errorf("title: length is lower than 1")
	}
	v.Target, err = webapi.DecodeConversationTargetJSON(fields["target"])
	if err != nil {
		return err
	}
	if string(fields["model"]) != "null" {
		value, err := core.Decode[core.ModelSelection](fields["model"])
		if err != nil {
			return err
		}
		v.Model = &value
	} else {
		v.Model = nil
	}
	if err = json.Unmarshal(fields["createdAt"], &v.CreatedAt); err != nil {
		return err
	}
	if err = json.Unmarshal(fields["attachedHosts"], &v.AttachedHosts); err != nil {
		return err
	}
	if v.AttachedHosts == nil {
		return fmt.Errorf("attachedHosts: expected an array")
	}
	return nil
}
func (v *CatalogRecord) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	fields, err := storedObject(dec, []string{"key", "checkedAt", "catalog"}, false)
	if err != nil {
		return err
	}
	if err = json.Unmarshal(fields["key"], &v.Key); err != nil {
		return err
	}
	if v.Key == "" {
		return fmt.Errorf("key: length is lower than 1")
	}
	if err = json.Unmarshal(fields["checkedAt"], &v.CheckedAt); err != nil {
		return err
	}
	v.Catalog, err = core.Decode[core.ProviderModelList](fields["catalog"])
	return err
}
