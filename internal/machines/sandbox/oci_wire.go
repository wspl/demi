package sandbox

// Only the fields written by the fixed profile are represented. Declaration
// order follows oci-spec 0.10.0's runtime structs, which fixes the JSON key order.
// +demi:root
type ociSpec struct {
	Version  string     `json:"ociVersion"`
	Root     ociRoot    `json:"root"`
	Mounts   []ociMount `json:"mounts"`
	Process  ociProcess `json:"process"`
	Hostname string     `json:"hostname"`
	Linux    ociLinux   `json:"linux"`
}

type ociRoot struct {
	Path     string `json:"path"`
	Readonly bool   `json:"readonly"`
}

type ociMount struct {
	Destination string   `json:"destination"`
	Type        string   `json:"type"`
	Source      string   `json:"source"`
	Options     []string `json:"options"`
}

type ociProcess struct {
	Terminal        bool            `json:"terminal"`
	User            ociUser         `json:"user"`
	Args            []string        `json:"args"`
	Env             []string        `json:"env"`
	Cwd             string          `json:"cwd"`
	Capabilities    ociCapabilities `json:"capabilities"`
	Rlimits         []ociRlimit     `json:"rlimits"`
	NoNewPrivileges bool            `json:"noNewPrivileges"`
}

type ociUser struct {
	UID   uint32 `json:"uid"`
	GID   uint32 `json:"gid"`
	Umask uint32 `json:"umask"`
}

type ociCapabilities struct {
	Bounding    []string `json:"bounding"`
	Effective   []string `json:"effective"`
	Inheritable []string `json:"inheritable"`
	Permitted   []string `json:"permitted"`
	Ambient     []string `json:"ambient"`
}

type ociRlimit struct {
	Type string `json:"type"`
	Hard uint64 `json:"hard"`
	Soft uint64 `json:"soft"`
}

type ociLinux struct {
	Resources     *ociResources  `json:"resources,omitempty"`
	CgroupsPath   *string        `json:"cgroupsPath,omitempty"`
	Namespaces    []ociNamespace `json:"namespaces"`
	MaskedPaths   []string       `json:"maskedPaths"`
	ReadonlyPaths []string       `json:"readonlyPaths"`
}

type ociResources struct {
	Memory ociMemory `json:"memory"`
	CPU    ociCPU    `json:"cpu"`
	Pids   ociPids   `json:"pids"`
}

type ociMemory struct {
	Limit int64 `json:"limit"`
	Swap  int64 `json:"swap"`
}

type ociCPU struct {
	Quota  int64  `json:"quota"`
	Period uint64 `json:"period"`
}

type ociPids struct {
	Limit int64 `json:"limit"`
}

type ociNamespace struct {
	Type string  `json:"type"`
	Path *string `json:"path,omitempty"`
}
