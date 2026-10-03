package live

import "github.com/wspl/demi/internal/cmdpkg/browser/browserop"

//go:generate go run github.com/wspl/demi/tools/contractgen

// +demi:root
// +demi:union tag=type
//
//sumtype:decl
type observerReport interface{ observerReport() }

// +demi:variant cursor
// +demi:tolerant
type cursorReport struct {
	Cursor   string `json:"cursor"`
	Editable bool   `json:"editable"`
}

func (*cursorReport) observerReport() {}

// +demi:variant controls
// +demi:tolerant
type controlsReport struct {
	Controls []browserop.LiveControl `json:"controls"`
}

func (*controlsReport) observerReport() {}

// +demi:variant copy
// +demi:tolerant
type copyReport struct {
	Text string `json:"text"`
}

func (*copyReport) observerReport() {}
