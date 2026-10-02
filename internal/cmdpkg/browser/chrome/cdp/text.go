package cdp

import (
	"bytes"
	"encoding/json"
	"fmt"
	"slices"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
)

// plain escapes page-controlled terminal and line controls inside browser output.
func plain(value string) string { return escaped(value, false, false) }

// quoted encloses page data using Rust's debug-string spelling.
func quoted(value string) string { return `"` + escaped(value, false, true) + `"` }

// body preserves page content's own lines and tabs without terminal controls.
func body(value string) string { return escaped(value, true, false) }

// escaped confines page-controlled strings to their documented browser text scope.
func escaped(value string, keepLines, quote bool) string {
	var out strings.Builder
	for _, r := range value {
		if quote && (r == '"' || r == '\\') {
			out.WriteRune('\\')
			out.WriteRune(r)
			continue
		}
		if !unicode.IsControl(r) || (keepLines && (r == '\n' || r == '\t')) {
			out.WriteRune(r)
			continue
		}
		switch r {
		case 0:
			out.WriteString(`\0`)
		case '\n':
			out.WriteString(`\n`)
		case '\r':
			out.WriteString(`\r`)
		case '\t':
			out.WriteString(`\t`)
		default:
			fmt.Fprintf(&out, `\u{%x}`, r)
		}
	}
	return out.String()
}

// pageValue renders a validated browser result value as text or compact JSON.
func pageValue(value any) string {
	if text, ok := value.(string); ok {
		return plain(text)
	}
	// The generated decoder admitted this JSON tree: no unsupported Go values,
	// invalid json.Number spellings or non-finite numbers can reach the encoder.
	raw, _ := Value(value)
	return string(raw)
}

// textObject is a rendering view of an already generated-decoder-validated result.
type textObject map[string]any

func (o textObject) text(key string) string {
	value, _ := o[key].(string)
	return value
}
func (o textObject) number(key string) string { return pageValue(o[key]) }
func (o textObject) flag(key string) bool {
	value, _ := o[key].(bool)
	return value
}
func (o textObject) object(key string) textObject {
	value, _ := o[key].(map[string]any)
	return textObject(value)
}
func (o textObject) list(key string) []any {
	value, _ := o[key].([]any)
	return value
}

// resultObject validates a Demi result through its generated decoder before text rendering.
func resultObject[T any](raw []byte, decode func([]byte) (T, error)) (textObject, error) {
	if _, err := decode(raw); err != nil {
		return nil, &BrowserError{Kind: KindInvalidResult, Message: err.Error(), Cause: err}
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.UseNumber()
	var object textObject
	if err := decoder.Decode(&object); err != nil {
		return nil, err
	}
	return object, nil
}

// textTable aligns browser result columns by Unicode scalar count, as Rust does.
func textTable(rows [][]string) string {
	if len(rows) == 0 {
		return ""
	}
	widths := make([]int, len(rows[0]))
	for _, row := range rows {
		for i, cell := range row {
			widths[i] = max(widths[i], utf8.RuneCountInString(cell))
		}
	}
	var text strings.Builder
	for _, row := range rows {
		var line strings.Builder
		for i, cell := range row {
			line.WriteString(cell)
			if i+1 < len(row) {
				line.WriteString(strings.Repeat(" ", widths[i]-utf8.RuneCountInString(cell)+2))
			}
		}
		text.WriteString(strings.TrimRightFunc(line.String(), unicode.IsSpace))
		text.WriteByte('\n')
	}
	return text.String()
}

// viewportText names the browser's CSS and device-pixel dimensions.
func viewportText(v textObject) string {
	return fmt.Sprintf("%s × %s CSS px, device pixel ratio %s, %s", v.number("width"), v.number("height"), v.number("devicePixelRatio"), v.text("mode"))
}

// dialogText quotes a browser dialog's page-controlled message.
func dialogText(v textObject) string {
	return fmt.Sprintf("Dialog: %s %s\n", v.text("type"), quoted(v.text("message")))
}

// nodeText names a browser tree node or flat match without allowing forged lines.
func nodeText(node textObject, tree bool) string {
	var line string
	if tree {
		line = "- "
		if role := node.text("role"); role != "" {
			line += plain(role)
		} else if tag := node.text("tag"); tag != "" {
			line += plain(tag)
		} else {
			line += "node"
		}
		if name, ok := node["name"].(string); ok && name != "" {
			line += " " + quoted(name)
		}
		if ref := node.text("ref"); ref != "" {
			line += " [ref=" + ref + "]"
		}
	} else {
		if ref := node.text("ref"); ref != "" {
			line = "[ref=" + ref + "] "
		}
		line += plain(node.text("role")) + " " + quoted(node.text("name"))
	}
	for _, state := range node.list("states") {
		line += " [" + plain(state.(string)) + "]"
	}
	if value, exists := node["value"]; exists {
		spelling := pageValue(value)
		if text, ok := value.(string); ok {
			spelling = quoted(text)
		}
		line += " [value=" + spelling + "]"
	}
	if tree && len(node.list("children")) > 0 {
		line += ":"
	}
	return line
}

// elementText names the actual browser element an action resolved.
func elementText(node textObject) string {
	parts := []string{}
	if node.text("role") != "" {
		parts = append(parts, plain(node.text("role")))
	}
	if node.text("name") != "" {
		parts = append(parts, quoted(node.text("name")))
	}
	parts = append(parts, "[ref="+node.text("ref")+"]")
	return strings.Join(parts, " ")
}

// requestedText describes the browser target in an upload invocation.
func requestedText(target *browserop.BrowserTarget) string {
	if target == nil {
		return ""
	}
	if target.Ref != nil {
		return "[ref=" + string(*target.Ref) + "]"
	}
	if target.Role != nil {
		value := plain(*target.Role)
		if target.Name != nil {
			value += " " + quoted(*target.Name)
		}
		return value
	}
	for _, field := range []struct {
		name  string
		value *string
	}{{"label", target.Label}, {"placeholder", target.Placeholder}, {"text", target.TextMatch}, {"test ID", target.TestID}, {"css", target.CSS}} {
		if field.value != nil {
			return field.name + " " + quoted(*field.value)
		}
	}
	return ""
}

// renderText renders each browser operation from the same validated result as JSON.
func renderText(operation browserop.Operation, raw []byte) (string, error) {
	input, ok := operation.(browserop.Input)
	if !ok {
		return "", &BrowserError{Kind: KindInvalidResult, Message: "operation has no finite browser result"}
	}
	name := input.OperationName()
	object, err := decodeTextResult(name, raw)
	if err != nil {
		return "", err
	}
	text := ""
	cut := true
	switch name {
	case "open":
		text = "Tab: " + object.text("tab") + "\nURL: " + plain(object.text("url")) + "\n"
		if title, ok := object["title"].(string); ok {
			text += "Title: " + plain(title) + "\n"
		}
	case "tabs":
		if len(object.list("tabs")) == 0 {
			return "No tabs.\n", nil
		}
		rows := [][]string{{"Tab", "Title", "Created by", "URL"}}
		for _, row := range object.list("tabs") {
			tab := textObject(row.(map[string]any))
			creator := tab.object("createdBy")
			by := creator.text("kind")
			switch by {
			case "agent", "temporary":
				by += " " + creator.number("number")
			case "page":
				by += " " + creator.text("opener")
			}
			rows = append(rows, []string{tab.text("id"), plain(tab.text("title")), by, plain(tab.text("url"))})
		}
		text = textTable(rows)
	case "info":
		text = fmt.Sprintf("Tab: %s · %s\nURL: %s\nViewport: %s\n", object.text("tab"), plain(object.text("title")), plain(object.text("url")), viewportText(object.object("viewport")))
		if dialog := object.object("dialog"); dialog != nil {
			text += dialogText(dialog)
		} else {
			text += "Dialog: none\n"
		}
	case "goto", "back", "forward", "reload":
		verb := "Navigated to"
		if name == "reload" {
			verb = "Reloaded"
		}
		text = verb + " " + plain(object.text("url")) + ".\n"
		if title, ok := object["title"].(string); ok {
			text += "Title: " + plain(title) + "\n"
		}
	case "history":
		rows := [][]string{}
		for _, row := range object.list("entries") {
			entry := textObject(row.(map[string]any))
			marker := " "
			if entry.flag("current") {
				marker = "*"
			}
			rows = append(rows, []string{marker + " " + entry.number("index"), plain(entry.text("title")), plain(entry.text("url"))})
		}
		text = textTable(rows)
		if len(rows) == 0 {
			text = "No history entries.\n"
		}
	case "close":
		text = "Closed " + object.text("closed") + ".\n"
	case "inspect":
		text = fmt.Sprintf("Tab: %s · %s\nURL: %s\n\n", object.text("tab"), plain(object.text("title")), plain(object.text("url")))
		type item struct {
			node  textObject
			depth int
		}
		pending := []item{}
		roots := object.list("tree")
		for i := len(roots) - 1; i >= 0; i-- {
			pending = append(pending, item{textObject(roots[i].(map[string]any)), 0})
		}
		for len(pending) > 0 {
			current := pending[len(pending)-1]
			pending = pending[:len(pending)-1]
			text += strings.Repeat("  ", min(current.depth, 64)) + nodeText(current.node, true) + "\n"
			children := current.node.list("children")
			for i := len(children) - 1; i >= 0; i-- {
				pending = append(pending, item{textObject(children[i].(map[string]any)), current.depth + 1})
			}
		}
		if len(roots) == 0 {
			text += "No nodes.\n"
		}
	case "find":
		count := object.number("count")
		if count == "0" {
			return "No matches.\n", nil
		}
		text = count + " matches:\n"
		if count == "1" {
			text = "1 match:\n"
		}
		for _, row := range object.list("matches") {
			text += "  " + nodeText(textObject(row.(map[string]any)), false) + "\n"
		}
	case "read":
		if value, exists := object["value"]; exists {
			return pageValue(value) + "\n", nil
		}
		for i, value := range object.list("values") {
			text += fmt.Sprintf("[%d] %s\n", i, pageValue(value))
		}
		if len(object.list("values")) == 0 {
			text = "No matches.\n"
		}
	case "screenshot":
		text = fmt.Sprintf("Screenshot saved: %s\nImage: %s × %s px, one pixel per CSS pixel\nViewport: %s\n", plain(object.text("path")), object.number("width"), object.number("height"), viewportText(object.object("viewport")))
	case "probe":
		for _, row := range object.list("matches") {
			node := textObject(row.(map[string]any))
			text += nodeText(node, false) + "\n"
			if bounds := node.object("bounds"); bounds != nil {
				text += fmt.Sprintf("Bounds: x=%s y=%s width=%s height=%s CSS px\n", bounds.number("x"), bounds.number("y"), bounds.number("width"), bounds.number("height"))
			}
		}
		if len(object.list("matches")) == 0 {
			text = "No nodes at that point.\n"
		}
		if path, ok := object["path"].(string); ok {
			text += "Annotated screenshot saved: " + plain(path) + "\n"
		}
	case "click", "move", "drag", "scroll", "fill", "type", "key", "check", "select", "select-text":
		return actionText(input, object)
	case "wait":
		condition := plain(object.text("condition"))
		if url, ok := object["url"].(string); ok {
			text = "URL matched: " + plain(url) + ".\n"
		} else if input.(*browserop.WaitInput).Load != nil {
			text = "Load state reached: " + condition + ".\n"
		} else if target := object.object("target"); target != nil {
			text = "Matched " + elementText(target) + ". State: " + condition + ".\n"
		} else {
			text = "No element matches. State: " + condition + ".\n"
		}
	case "upload":
		files := "files"
		if object.number("attached") == "1" {
			files = "file"
		}
		through := requestedText(input.ElementTarget())
		if through != "" {
			through = " through " + through
		}
		text = "Attached " + object.number("attached") + " " + files + through + ".\n"
		for _, file := range object.list("files") {
			text += "  " + plain(file.(string)) + "\n"
		}
	case "download":
		text = "Downloaded: " + plain(object.text("path")) + "\n"
		if suggested := object.text("suggestedFilename"); suggested != "" {
			text += "Suggested filename: " + plain(suggested) + "\n"
		}
		text += "Bytes: " + object.number("bytes") + "\n"
	case "clipboard.write":
		text = "Clipboard written: " + object.text("mimeType") + ", " + object.number("bytes") + " bytes.\n"
	case "clipboard.read":
		if value, ok := object["text"].(string); ok {
			text = body(value) + "\n"
		} else {
			text = "Clipboard exported:\n"
			for _, row := range object.list("items") {
				item := textObject(row.(map[string]any))
				text += "  " + item.text("mimeType") + ": " + plain(item.text("path")) + "\n"
			}
		}
	case "eval":
		encoded, err := Value(object["value"])
		if err != nil {
			return "", err
		}
		text = string(encoded) + "\n"
	case "logs":
		for _, row := range object.list("entries") {
			entry := textObject(row.(map[string]any))
			text += "[" + entry.text("level") + "] " + plain(entry.text("text")) + "\n"
			if url, ok := entry["url"].(string); ok {
				text += "  " + plain(url) + "\n"
			}
		}
		if len(object.list("entries")) == 0 {
			text = "No log entries.\n"
		}
		text += "Cursor: " + plain(object.text("cursor")) + "\n"
		if object.flag("hasMore") {
			text += "Has more: true\n"
		}
	case "viewport.set", "viewport.reset":
		text = "Viewport: " + viewportText(object.object("viewport")) + ".\n"
	case "dialog.inspect":
		if dialog := object.object("dialog"); dialog != nil {
			text = "Dialog: " + dialog.text("type") + "\nMessage: " + quoted(dialog.text("message")) + "\n"
		} else {
			text = "No dialog.\n"
		}
	case "dialog.accept", "dialog.dismiss":
		outcome := "Accepted"
		if object.text("outcome") == "dismissed" {
			outcome = "Dismissed"
		}
		text = outcome + " " + object.text("type") + " dialog.\n"
	case "cdp.targets":
		rows := [][]string{{"Target", "Kind", "URL"}}
		for _, row := range object.list("targets") {
			target := textObject(row.(map[string]any))
			rows = append(rows, []string{plain(target.text("id")), plain(target.text("kind")), plain(target.text("url"))})
		}
		text = textTable(rows)
	case "cdp.detach":
		text = "Detached debugging from " + object.text("detached") + ".\n"
	case "cdp.send", "webmcp.call":
		result, err := Value(object["result"])
		if err != nil {
			return "", err
		}
		if name == "cdp.send" {
			text = "CDP " + plain(object.text("method")) + " completed.\n"
		} else {
			text = "Called " + plain(object.text("name")) + ".\n"
		}
		text += "Result: " + string(result) + "\n"
	case "cdp.events":
		cursor := plain(object.text("cursor"))
		if len(object.list("events")) == 0 {
			return "Cursor: " + cursor + "\nNo events. Use --after " + cursor + " to read subsequent events.\n", nil
		}
		for _, row := range object.list("events") {
			event := textObject(row.(map[string]any))
			text += "[" + event.number("sequence") + "] " + plain(event.text("method")) + "\n"
			if params := event.object("params"); params != nil {
				keys := []string{}
				for key := range params {
					keys = append(keys, key)
				}
				slices.Sort(keys)
				for _, key := range keys {
					text += "  " + plain(key) + ": " + pageValue(params[key]) + "\n"
				}
			} else {
				encoded, err := Value(event["params"])
				if err != nil {
					return "", err
				}
				text += "  " + string(encoded) + "\n"
			}
		}
		text += fmt.Sprintf("Cursor: %s\nHas more: %t\nTruncated: %t\n", cursor, object.flag("hasMore"), object.flag("truncated"))
		cut = false
	case "content.read":
		if path, ok := object["path"].(string); ok {
			text = "Content saved: " + plain(path) + "\nFormat: " + object.text("format") + "\n"
		} else {
			text = "Title: " + plain(object.text("title")) + "\nURL: " + plain(object.text("url")) + "\n\n" + body(object.text("content")) + "\n"
		}
	case "content.fetch":
		pages := []string{}
		for _, row := range object.list("pages") {
			page := textObject(row.(map[string]any))
			url := page.text("url")
			if url == "" {
				url = page.text("requestedUrl")
			}
			part := "URL: " + plain(url) + "\n"
			if failure := page.object("error"); failure != nil {
				part += "Error: " + failure.text("code") + "\n" + plain(failure.text("message")) + "\n"
			} else {
				part += "Title: " + plain(page.text("title")) + "\n" + body(page.text("content")) + "\n"
			}
			pages = append(pages, part)
		}
		text = strings.Join(pages, "\n")
	case "assets.list":
		text = "Inventory: " + plain(object.text("inventory")) + "\n"
		rows := [][]string{}
		for _, row := range object.list("assets") {
			asset := textObject(row.(map[string]any))
			rows = append(rows, []string{" " + plain(asset.text("id")), asset.text("kind"), plain(asset.text("url"))})
		}
		text += textTable(rows)
		for _, row := range object.list("inlineSvgs") {
			svg := textObject(row.(map[string]any))
			text += " " + plain(svg.text("id")) + "  inline svg\n"
		}
		if len(rows) == 0 && len(object.list("inlineSvgs")) == 0 {
			text += "No assets.\n"
		}
	case "assets.export":
		count := len(object.list("files"))
		noun := "assets"
		if count == 1 {
			noun = "asset"
		}
		text = fmt.Sprintf("Exported %d %s to %s.\nManifest: %s\n", count, noun, plain(object.text("directory")), plain(object.text("manifest")))
	case "capabilities":
		available, unavailable := "", ""
		for _, row := range object.list("capabilities") {
			capability := textObject(row.(map[string]any))
			line := "  " + plain(capability.text("id"))
			if capability.flag("available") {
				available += line + "\n"
			} else {
				if reason, ok := capability["reason"].(string); ok {
					line += " — " + plain(reason)
				}
				unavailable += line + "\n"
			}
		}
		if available != "" {
			text = "Available:\n" + available
		}
		if unavailable != "" {
			text += "Unavailable:\n" + unavailable
		}
	case "webmcp.list":
		text = "Tools: " + plain(object.text("tools")) + "\n"
		for _, row := range object.list("entries") {
			tool := textObject(row.(map[string]any))
			schema, err := Value(tool["inputSchema"])
			if err != nil {
				return "", err
			}
			text += "  " + plain(tool.text("name")) + " — " + plain(tool.text("description")) + "\n    Input: " + string(schema) + "\n"
		}
		if len(object.list("entries")) == 0 {
			text += "No tools.\n"
		}
	}
	if cut && object.flag("truncated") {
		text += "[truncated]\n"
	}
	return text, nil
}

// actionText describes input actually delivered to a browser element or document.
func actionText(input browserop.Input, result textObject) (string, error) {
	target := ""
	if element := result.object("target"); element != nil {
		target = elementText(element)
	} else if input.OperationName() == "type" || input.OperationName() == "key" {
		target = "the document"
	}
	on := func(verb string) string {
		if target != "" {
			return verb + " " + target + "."
		}
		return verb + "."
	}
	text := ""
	switch input := input.(type) {
	case *browserop.ClickInput:
		verb := "Clicked"
		if input.Count != nil && *input.Count == 2 {
			verb = "Double-clicked"
		} else if input.Button != nil && *input.Button == "right" {
			verb = "Right-clicked"
		} else if input.Button != nil && *input.Button == "middle" {
			verb = "Middle-clicked"
		}
		if target == "" && input.XY != nil {
			text = verb + " at " + plain(*input.XY) + "."
		} else {
			text = on(verb)
		}
	case *browserop.MoveInput:
		if target == "" && input.XY != nil {
			text = "Pointer moved to " + plain(*input.XY) + "."
		} else {
			text = on("Pointer moved to")
		}
	case *browserop.DragInput:
		if len(input.Point) > 0 {
			text = "Dragged from " + plain(string(input.Point[0])) + " to " + plain(string(input.Point[len(input.Point)-1])) + "."
		} else {
			text = "Dragged."
		}
	case *browserop.ScrollInput:
		amounts := []string{}
		if input.Dx != nil {
			amounts = append(amounts, fmt.Sprintf("dx=%v", *input.Dx))
		}
		if input.Dy != nil {
			amounts = append(amounts, fmt.Sprintf("dy=%v", *input.Dy))
		}
		place := ""
		if target != "" {
			place = " to " + target
		}
		text = "Scroll input delivered" + place + ": " + strings.Join(amounts, " ") + "."
	case *browserop.FillInput:
		text = on("Filled")
	case *browserop.TypeInput:
		if target != "" {
			text = "Typed into " + target + "."
		} else {
			text = "Typed."
		}
	case *browserop.KeyInput:
		if target != "" {
			text = "Pressed " + plain(input.Key) + " in " + target + "."
		} else {
			text = "Pressed " + plain(input.Key) + "."
		}
	case *browserop.CheckInput:
		if input.Value {
			text = on("Checked")
		} else {
			text = on("Unchecked")
		}
	case *browserop.SelectInput:
		raw, err := Value(result["result"])
		if err != nil {
			return "", err
		}
		var entries []json.RawMessage
		if err := json.Unmarshal(raw, &entries); err != nil {
			return "", err
		}
		selected := []string{}
		for _, raw := range entries {
			item, err := browserop.DecodeSelectedOption(raw)
			if err != nil {
				return "", err
			}
			value := plain(item.Value)
			if item.Label != "" && item.Label != item.Value {
				value = plain(item.Label) + " (" + value + ")"
			}
			selected = append(selected, value)
		}
		text = "Selected: " + strings.Join(selected, ", ") + "."
	case *browserop.SelectTextInput:
		place := ""
		if target != "" {
			place = " in " + target
		}
		switch result.text("result") {
		case "before":
			text = "Cursor placed before the matching text" + place + "."
		case "after":
			text = "Cursor placed after the matching text" + place + "."
		default:
			text = "Selected text" + place + "."
		}
	default:
		text = plain(result.text("operation")) + " completed."
	}
	text += "\n"
	if url, ok := result["url"].(string); ok {
		text += "URL: " + plain(url) + "\n"
	}
	for _, tab := range result.list("openedTabs") {
		text += "Opened tab: " + tab.(string) + "\n"
	}
	if dialog := result.object("dialog"); dialog != nil {
		text += dialogText(dialog)
	}
	return text, nil
}

// decodeTextResult selects the authoritative generated browser result decoder.
func decodeTextResult(name string, raw []byte) (textObject, error) {
	switch name {
	case "open":
		return resultObject(raw, browserop.DecodeOpenResult)
	case "tabs":
		return resultObject(raw, browserop.DecodeTabsResult)
	case "info":
		return resultObject(raw, browserop.DecodeInfoResult)
	case "goto":
		return resultObject(raw, browserop.DecodeNavigationResult)
	case "back":
		return resultObject(raw, browserop.DecodeNavigationResult)
	case "forward":
		return resultObject(raw, browserop.DecodeNavigationResult)
	case "reload":
		return resultObject(raw, browserop.DecodeNavigationResult)
	case "history":
		return resultObject(raw, browserop.DecodeHistoryResult)
	case "close":
		return resultObject(raw, browserop.DecodeCloseResult)
	case "inspect":
		return resultObject(raw, browserop.DecodeInspectResult)
	case "find":
		return resultObject(raw, browserop.DecodeFindResult)
	case "read":
		return resultObject(raw, browserop.DecodeReadResult)
	case "screenshot":
		return resultObject(raw, browserop.DecodeScreenshotResult)
	case "probe":
		return resultObject(raw, browserop.DecodeProbeResult)
	case "wait":
		return resultObject(raw, browserop.DecodeWaitResult)
	case "upload":
		return resultObject(raw, browserop.DecodeUploadResult)
	case "download":
		return resultObject(raw, browserop.DecodeDownloadResult)
	case "clipboard.write":
		return resultObject(raw, browserop.DecodeClipboardWriteResult)
	case "clipboard.read":
		return resultObject(raw, browserop.DecodeClipboardReadResult)
	case "eval":
		return resultObject(raw, browserop.DecodeEvalResult)
	case "logs":
		return resultObject(raw, browserop.DecodeLogsResult)
	case "viewport.set":
		return resultObject(raw, browserop.DecodeViewportResult)
	case "viewport.reset":
		return resultObject(raw, browserop.DecodeViewportResult)
	case "dialog.inspect":
		return resultObject(raw, browserop.DecodeDialogInspectResult)
	case "dialog.accept":
		return resultObject(raw, browserop.DecodeDialogResult)
	case "dialog.dismiss":
		return resultObject(raw, browserop.DecodeDialogResult)
	case "cdp.targets":
		return resultObject(raw, browserop.DecodeCdpTargetsResult)
	case "cdp.detach":
		return resultObject(raw, browserop.DecodeCdpDetachResult)
	case "cdp.send":
		return resultObject(raw, browserop.DecodeCdpSendResult)
	case "cdp.events":
		return resultObject(raw, browserop.DecodeCdpEventsResult)
	case "content.read":
		return resultObject(raw, browserop.DecodeContentReadResult)
	case "content.fetch":
		return resultObject(raw, browserop.DecodeContentFetchResult)
	case "assets.list":
		return resultObject(raw, browserop.DecodeAssetsListResult)
	case "assets.export":
		return resultObject(raw, browserop.DecodeAssetsExportResult)
	case "capabilities":
		return resultObject(raw, browserop.DecodeCapabilitiesResult)
	case "webmcp.list":
		return resultObject(raw, browserop.DecodeWebmcpListResult)
	case "webmcp.call":
		return resultObject(raw, browserop.DecodeWebmcpCallResult)
	case "click":
		return resultObject(raw, browserop.DecodeActionResult)
	case "move":
		return resultObject(raw, browserop.DecodeActionResult)
	case "drag":
		return resultObject(raw, browserop.DecodeActionResult)
	case "scroll":
		return resultObject(raw, browserop.DecodeActionResult)
	case "fill":
		return resultObject(raw, browserop.DecodeActionResult)
	case "type":
		return resultObject(raw, browserop.DecodeActionResult)
	case "key":
		return resultObject(raw, browserop.DecodeActionResult)
	case "check":
		return resultObject(raw, browserop.DecodeActionResult)
	case "select":
		return resultObject(raw, browserop.DecodeActionResult)
	case "select-text":
		return resultObject(raw, browserop.DecodeActionResult)
	default:
		return nil, &BrowserError{Kind: KindInvalidResult, Message: "operation has no finite browser result"}
	}
}
