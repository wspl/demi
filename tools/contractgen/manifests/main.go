// Command manifests prints page manifests in backend registration order.
package main

import (
	"encoding/json"
	"fmt"
	"io"
	"os"

	"github.com/wspl/demi/internal/backend"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/tools/contractgen/pagemeta"
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run() error {
	factories, err := backend.BuiltinPlugins()
	if err != nil {
		return err
	}
	return writePages(os.Stdout, factories)
}

func writePages(output io.Writer, factories []plugin.Factory) error {
	pages := make([]pagemeta.Page, 0)
	for _, factory := range factories {
		manifest := factory.Manifest()
		if manifest.Page != nil {
			pages = append(pages, metadata(manifest))
		}
	}
	encoder := json.NewEncoder(output)
	encoder.SetEscapeHTML(false)
	return encoder.Encode(pages)
}

func metadata(manifest plugin.Manifest) pagemeta.Page {
	page := manifest.Page
	result := pagemeta.Page{ID: string(manifest.ID), Package: page.Package, Schemas: []pagemeta.Schema{}, Constants: []pagemeta.Constant{}}
	for _, state := range []*plugin.State{page.User, page.Conversation} {
		if state != nil {
			result.Schemas = append(result.Schemas, pagemeta.Schema{Direction: "receive", Value: state.Schema.Document()})
		}
	}
	for _, method := range page.Methods {
		result.Schemas = append(result.Schemas,
			pagemeta.Schema{Direction: "send", Value: method.Params.Document()},
			pagemeta.Schema{Direction: "receive", Value: method.Result.Document()})
	}
	for _, stream := range manifest.Streams {
		result.Schemas = append(result.Schemas,
			pagemeta.Schema{Direction: "receive", Value: stream.Receives.Document()},
			pagemeta.Schema{Direction: "send", Value: stream.Sends.Document()})
		for _, constant := range stream.Constants {
			result.Constants = append(result.Constants, pagemeta.Constant{Name: constant.Name, Description: constant.Description, Value: constant.Value})
		}
	}
	return result
}
