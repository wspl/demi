package cdp

import (
	"bytes"
	"encoding/json"
	"fmt"
	"slices"
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
)

// plain escapes page-controlled terminal and line controls inside browser output.
func plain(value string) string { return escaped(value, false) }

// quoted encloses page data in Go's quoted-string syntax (strconv.Quote).
func quoted(value string) string { return strconv.Quote(value) }

// body preserves page content's own lines and tabs without terminal controls.
func body(value string) string { return escaped(value, true) }

// escaped confines page-controlled strings to their documented
// browser text scope, escaping control characters as Go's quoted strings do.
func escaped(value string, keepLines bool) string {
	var out strings.Builder
	for _, r := range value {
		if !unicode.IsControl(r) || (keepLines && (r == '\n' || r == '\t')) {
			out.WriteRune(r)
			continue
		}
		quotedRune := strconv.Quote(string(r))
		out.WriteString(quotedRune[1 : len(quotedRune)-1])
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

// textTable aligns browser result columns by Unicode scalar count, not bytes or display width.
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
	return fmt.Sprintf(
		"%s × %s CSS px, device pixel ratio %s, %s",
		v.number("width"),
		v.number("height"),
		v.number("devicePixelRatio"),
		v.text("mode"),
	)
}

// dialogText quotes a browser dialog's page-controlled message.
func dialogText(v textObject) string {
	return fmt.Sprintf("Dialog: %s %s\n", v.text("type"), quoted(v.text("message")))
}

// nodeText names a browser tree node or flat match without allowing forged lines.
func nodeText(node textObject, tree bool) string {
	var line string
	if tree {
		line = treeNodeHeading(node)
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
func requestedText(target browserproto.BrowserTarget) string {
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
	}{{
		"label",
		target.Label,
	}, {
		"placeholder",
		target.Placeholder,
	}, {
		"text",
		target.TextMatch,
	}, {
		"test ID",
		target.TestID,
	}, {
		"css",
		target.CSS,
	}} {
		if field.value != nil {
			return field.name + " " + quoted(*field.value)
		}
	}
	return ""
}

// renderText renders each browser operation from the same validated result as JSON.
func renderText(operation browserproto.Operation, raw []byte) (string, error) {
	input, ok := operation.(browserproto.Input)
	if !ok {
		return "", &BrowserError{Kind: KindInvalidResult, Message: "operation has no finite browser result"}
	}
	name := input.OperationName()
	object, err := decodeTextResult(name, raw)
	if err != nil {
		return "", err
	}
	return renderPageText(input, object)
}

// actionText describes input actually delivered to a browser element or document.
func actionText(input browserproto.Input, result textObject) (string, error) {
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
	text, err := actionSummary(input, result, target, on)
	if err != nil {
		return "", err
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
		return resultObject(raw, browserproto.DecodeOpenResult)
	case "tabs":
		return resultObject(raw, browserproto.DecodeTabsResult)
	case "info":
		return resultObject(raw, browserproto.DecodeInfoResult)
	case "goto", "back", "forward", "reload":
		return resultObject(raw, browserproto.DecodeNavigationResult)
	case "history":
		return resultObject(raw, browserproto.DecodeHistoryResult)
	case "close":
		return resultObject(raw, browserproto.DecodeCloseResult)
	case "inspect":
		return resultObject(raw, browserproto.DecodeInspectResult)
	case "find":
		return resultObject(raw, browserproto.DecodeFindResult)
	case "read":
		return resultObject(raw, browserproto.DecodeReadResult)
	case "screenshot":
		return resultObject(raw, browserproto.DecodeScreenshotResult)
	case "probe":
		return resultObject(raw, browserproto.DecodeProbeResult)
	case "wait":
		return resultObject(raw, browserproto.DecodeWaitResult)
	case "upload":
		return resultObject(raw, browserproto.DecodeUploadResult)
	case "download":
		return resultObject(raw, browserproto.DecodeDownloadResult)
	case "clipboard.write":
		return resultObject(raw, browserproto.DecodeClipboardWriteResult)
	case "clipboard.read":
		return resultObject(raw, browserproto.DecodeClipboardReadResult)
	case "eval":
		return resultObject(raw, browserproto.DecodeEvalResult)
	case "logs":
		return resultObject(raw, browserproto.DecodeLogsResult)
	default:
		return decodeControlTextResult(name, raw)
	}
}

func renderOpenText(object textObject) (string, error) {
	text := ""
	text = "Tab: " + object.text("tab") + "\nURL: " + plain(object.text("url")) + "\n"
	if title, ok := object["title"].(string); ok {
		text += "Title: " + plain(title) + "\n"
	}

	return truncatedText(text, object)
}

func renderTabsText(object textObject) (string, error) {
	text := ""
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

	return truncatedText(text, object)
}

func renderInfoText(object textObject) (string, error) {
	text := ""
	text = fmt.Sprintf(
		"Tab: %s · %s\nURL: %s\nViewport: %s\n",
		object.text("tab"),
		plain(object.text("title")),
		plain(object.text("url")),
		viewportText(object.object("viewport")),
	)
	if dialog := object.object("dialog"); dialog != nil {
		text += dialogText(dialog)
	} else {
		text += "Dialog: none\n"
	}

	return truncatedText(text, object)
}

func renderGotoText(input browserproto.Input, object textObject) (string, error) {
	text := ""
	name := input.OperationName()
	verb := "Navigated to"
	if name == "reload" {
		verb = "Reloaded"
	}
	text = verb + " " + plain(object.text("url")) + ".\n"
	if title, ok := object["title"].(string); ok {
		text += "Title: " + plain(title) + "\n"
	}

	return truncatedText(text, object)
}

func renderHistoryText(object textObject) (string, error) {
	text := ""
	rows := [][]string{}
	for _, row := range object.list("entries") {
		entry := textObject(row.(map[string]any))
		marker := " "
		if entry.flag("current") {
			marker = "*"
		}
		rows = append(
			rows,
			[]string{marker + " " + entry.number("index"), plain(entry.text("title")), plain(entry.text("url"))},
		)
	}
	text = textTable(rows)
	if len(rows) == 0 {
		text = "No history entries.\n"
	}

	return truncatedText(text, object)
}

func renderCloseText(object textObject) (string, error) {
	text := ""
	text = "Closed " + object.text("closed") + ".\n"

	return truncatedText(text, object)
}

func renderInspectText(object textObject) (string, error) {
	text := ""
	text = fmt.Sprintf(
		"Tab: %s · %s\nURL: %s\n\n",
		object.text("tab"),
		plain(object.text("title")),
		plain(object.text("url")),
	)
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

	return truncatedText(text, object)
}

func renderFindText(object textObject) (string, error) {
	text := ""
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

	return truncatedText(text, object)
}

func renderReadText(object textObject) (string, error) {
	text := ""
	if value, exists := object["value"]; exists {
		return pageValue(value) + "\n", nil
	}
	for i, value := range object.list("values") {
		text += fmt.Sprintf("[%d] %s\n", i, pageValue(value))
	}
	if len(object.list("values")) == 0 {
		text = "No matches.\n"
	}

	return truncatedText(text, object)
}

func renderScreenshotText(object textObject) (string, error) {
	text := ""
	text = fmt.Sprintf(
		"Screenshot saved: %s\nImage: %s × %s px, one pixel per CSS pixel\nViewport: %s\n",
		plain(object.text("path")),
		object.number("width"),
		object.number("height"),
		viewportText(object.object("viewport")),
	)

	return truncatedText(text, object)
}

func renderProbeText(object textObject) (string, error) {
	text := ""
	for _, row := range object.list("matches") {
		node := textObject(row.(map[string]any))
		text += nodeText(node, false) + "\n"
		if bounds := node.object("bounds"); bounds != nil {
			text += fmt.Sprintf(
				"Bounds: x=%s y=%s width=%s height=%s CSS px\n",
				bounds.number("x"),
				bounds.number("y"),
				bounds.number("width"),
				bounds.number("height"),
			)
		}
	}
	if len(object.list("matches")) == 0 {
		text = "No nodes at that point.\n"
	}
	if path, ok := object["path"].(string); ok {
		text += "Annotated screenshot saved: " + plain(path) + "\n"
	}

	return truncatedText(text, object)
}

func renderWaitText(input browserproto.Input, object textObject) (string, error) {
	text := ""
	condition := plain(object.text("condition"))
	if url, ok := object["url"].(string); ok {
		text = "URL matched: " + plain(url) + ".\n"
	} else if input.(*browserproto.WaitInput).Load != nil {
		text = "Load state reached: " + condition + ".\n"
	} else if target := object.object("target"); target != nil {
		text = "Matched " + elementText(target) + ". State: " + condition + ".\n"
	} else {
		text = "No element matches. State: " + condition + ".\n"
	}

	return truncatedText(text, object)
}

func renderUploadText(input browserproto.Input, object textObject) (string, error) {
	text := ""
	files := "files"
	if object.number("attached") == "1" {
		files = "file"
	}
	through := ""
	if target, ok := input.ElementTarget(); ok {
		through = requestedText(target)
	}
	if through != "" {
		through = " through " + through
	}
	text = "Attached " + object.number("attached") + " " + files + through + ".\n"
	for _, file := range object.list("files") {
		text += "  " + plain(file.(string)) + "\n"
	}

	return truncatedText(text, object)
}

func renderDownloadText(object textObject) (string, error) {
	text := ""
	text = "Downloaded: " + plain(object.text("path")) + "\n"
	if suggested := object.text("suggestedFilename"); suggested != "" {
		text += "Suggested filename: " + plain(suggested) + "\n"
	}
	text += "Bytes: " + object.number("bytes") + "\n"

	return truncatedText(text, object)
}

func renderClipboardWriteText(object textObject) (string, error) {
	text := ""
	text = "Clipboard written: " + object.text("mimeType") + ", " + object.number("bytes") + " bytes.\n"

	return truncatedText(text, object)
}

func renderClipboardReadText(object textObject) (string, error) {
	text := ""
	if value, ok := object["text"].(string); ok {
		text = body(value) + "\n"
	} else {
		text = "Clipboard exported:\n"
		for _, row := range object.list("items") {
			item := textObject(row.(map[string]any))
			text += "  " + item.text("mimeType") + ": " + plain(item.text("path")) + "\n"
		}
	}

	return truncatedText(text, object)
}

func renderEvalText(object textObject) (string, error) {
	text := ""
	encoded, err := Value(object["value"])
	if err != nil {
		return "", err
	}
	text = string(encoded) + "\n"

	return truncatedText(text, object)
}

func renderLogsText(object textObject) (string, error) {
	text := ""
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

	return truncatedText(text, object)
}

func renderViewportSetText(object textObject) (string, error) {
	text := ""
	text = "Viewport: " + viewportText(object.object("viewport")) + ".\n"

	return truncatedText(text, object)
}

func renderDialogInspectText(object textObject) (string, error) {
	text := ""
	if dialog := object.object("dialog"); dialog != nil {
		text = "Dialog: " + dialog.text("type") + "\nMessage: " + quoted(dialog.text("message")) + "\n"
	} else {
		text = "No dialog.\n"
	}

	return truncatedText(text, object)
}

func renderDialogAcceptText(object textObject) (string, error) {
	text := ""
	outcome := "Accepted"
	if object.text("outcome") == "dismissed" {
		outcome = "Dismissed"
	}
	text = outcome + " " + object.text("type") + " dialog.\n"

	return truncatedText(text, object)
}

func renderCDPTargetsText(object textObject) (string, error) {
	text := ""
	rows := [][]string{{"Target", "Kind", "URL"}}
	for _, row := range object.list("targets") {
		target := textObject(row.(map[string]any))
		rows = append(
			rows,
			[]string{plain(target.text("id")), plain(target.text("kind")), plain(target.text("url"))},
		)
	}
	text = textTable(rows)

	return truncatedText(text, object)
}

func renderCDPDetachText(object textObject) (string, error) {
	text := ""
	text = "Detached debugging from " + object.text("detached") + ".\n"

	return truncatedText(text, object)
}

func renderCDPSendText(input browserproto.Input, object textObject) (string, error) {
	text := ""
	name := input.OperationName()
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

	return truncatedText(text, object)
}

func renderCDPEventsText(object textObject) (string, error) {
	text := ""
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
	text += fmt.Sprintf(
		"Cursor: %s\nHas more: %t\nTruncated: %t\n",
		cursor,
		object.flag("hasMore"),
		object.flag("truncated"),
	)

	return text, nil
}

func renderContentReadText(object textObject) (string, error) {
	text := ""
	if path, ok := object["path"].(string); ok {
		text = "Content saved: " + plain(path) + "\nFormat: " + object.text("format") + "\n"
	} else {
		text = "Title: " + plain(
			object.text("title"),
		) + "\nURL: " + plain(
			object.text("url"),
		) + "\n\n" + body(
			object.text("content"),
		) + "\n"
	}

	return truncatedText(text, object)
}

func renderContentFetchText(object textObject) (string, error) {
	text := ""
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

	return truncatedText(text, object)
}

func renderAssetsListText(object textObject) (string, error) {
	text := ""
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

	return truncatedText(text, object)
}

func renderAssetsExportText(object textObject) (string, error) {
	text := ""
	count := len(object.list("files"))
	noun := "assets"
	if count == 1 {
		noun = "asset"
	}
	text = fmt.Sprintf(
		"Exported %d %s to %s.\nManifest: %s\n",
		count,
		noun,
		plain(object.text("directory")),
		plain(object.text("manifest")),
	)

	return truncatedText(text, object)
}

func renderCapabilitiesText(object textObject) (string, error) {
	text := ""
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

	return truncatedText(text, object)
}

func renderWebMCPListText(object textObject) (string, error) {
	text := ""
	text = "Tools: " + plain(object.text("tools")) + "\n"
	for _, row := range object.list("entries") {
		tool := textObject(row.(map[string]any))
		schema, err := Value(tool["inputSchema"])
		if err != nil {
			return "", err
		}
		text += "  " + plain(
			tool.text("name"),
		) + " — " + plain(
			tool.text("description"),
		) + "\n    Input: " + string(
			schema,
		) + "\n"
	}
	if len(object.list("entries")) == 0 {
		text += "No tools.\n"
	}
	return truncatedText(text, object)
}

func selectedOptionsText(result textObject) (string, error) {
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
		item, err := browserproto.DecodeSelectedOption(raw)
		if err != nil {
			return "", err
		}
		value := plain(item.Value)
		if item.Label != "" && item.Label != item.Value {
			value = plain(item.Label) + " (" + value + ")"
		}
		selected = append(selected, value)
	}
	text := "Selected: " + strings.Join(selected, ", ") + "."
	return text, nil
}

func treeNodeHeading(node textObject) string {
	line := ""
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

	return line
}

func decodeExtendedTextResult(name string, raw []byte) (textObject, error) {
	switch name {
	case "cdp.targets":
		return resultObject(raw, browserproto.DecodeCDPTargetsResult)
	case "cdp.detach":
		return resultObject(raw, browserproto.DecodeCDPDetachResult)
	case "cdp.send":
		return resultObject(raw, browserproto.DecodeCDPSendResult)
	case "cdp.events":
		return resultObject(raw, browserproto.DecodeCDPEventsResult)
	case "content.read":
		return resultObject(raw, browserproto.DecodeContentReadResult)
	case "content.fetch":
		return resultObject(raw, browserproto.DecodeContentFetchResult)
	case "assets.list":
		return resultObject(raw, browserproto.DecodeAssetsListResult)
	case "assets.export":
		return resultObject(raw, browserproto.DecodeAssetsExportResult)
	case "capabilities":
		return resultObject(raw, browserproto.DecodeCapabilitiesResult)
	case "webmcp.list":
		return resultObject(raw, browserproto.DecodeWebMCPListResult)
	case "webmcp.call":
		return resultObject(raw, browserproto.DecodeWebMCPCallResult)
	case "click", "move", "drag", "scroll", "fill", "type", "key", "check", "select", "select-text":
		return resultObject(raw, browserproto.DecodeActionResult)
	default:
		return nil, &BrowserError{Kind: KindInvalidResult, Message: "operation has no finite browser result"}
	}
}

func renderPageText(input browserproto.Input, object textObject) (string, error) {
	switch input.OperationName() {
	case "open":
		return renderOpenText(object)
	case "tabs":
		return renderTabsText(object)
	case "info":
		return renderInfoText(object)
	case "goto", "back", "forward", "reload":
		return renderGotoText(input, object)
	case "history":
		return renderHistoryText(object)
	case "close":
		return renderCloseText(object)
	case "inspect":
		return renderInspectText(object)
	case "find":
		return renderFindText(object)
	case "read":
		return renderReadText(object)
	case "screenshot":
		return renderScreenshotText(object)
	case "probe":
		return renderProbeText(object)
	case "click", "move", "drag", "scroll", "fill", "type", "key", "check", "select", "select-text":
		return actionText(input, object)
	case "wait":
		return renderWaitText(input, object)
	default:
		return renderActionOutcomeText(input, object)
	}
}

func renderExtendedText(input browserproto.Input, object textObject) (string, error) {
	switch input.OperationName() {
	case "cdp.targets":
		return renderCDPTargetsText(object)
	case "cdp.detach":
		return renderCDPDetachText(object)
	case "cdp.send", "webmcp.call":
		return renderCDPSendText(input, object)
	case "cdp.events":
		return renderCDPEventsText(object)
	case "content.read":
		return renderContentReadText(object)
	case "content.fetch":
		return renderContentFetchText(object)
	case "assets.list":
		return renderAssetsListText(object)
	case "assets.export":
		return renderAssetsExportText(object)
	case "capabilities":
		return renderCapabilitiesText(object)
	case "webmcp.list":
		return renderWebMCPListText(object)
	}
	return truncatedText("", object)
}

func truncatedText(text string, object textObject) (string, error) {
	if object.flag("truncated") {
		text += "[truncated]\n"
	}
	return text, nil
}

func decodeControlTextResult(name string, raw []byte) (textObject, error) {
	switch name {
	case "viewport.set", "viewport.reset":
		return resultObject(raw, browserproto.DecodeViewportResult)
	case "dialog.inspect":
		return resultObject(raw, browserproto.DecodeDialogInspectResult)
	case "dialog.accept", "dialog.dismiss":
		return resultObject(raw, browserproto.DecodeDialogResult)
	default:
		return decodeExtendedTextResult(name, raw)
	}
}

func actionSummary(input browserproto.Input, result textObject, target string, on func(string) string) (string, error) {
	text := ""
	switch input := input.(type) {
	case *browserproto.ClickInput:
		text = clickSummary(input, target, on)
	case *browserproto.MoveInput:
		if target == "" && input.XY != nil {
			text = "Pointer moved to " + plain(*input.XY) + "."
		} else {
			text = on("Pointer moved to")
		}
	case *browserproto.DragInput:
		if len(input.Point) > 0 {
			text = "Dragged from " + plain(
				string(input.Point[0]),
			) + " to " + plain(
				string(input.Point[len(input.Point)-1]),
			) + "."
		} else {
			text = "Dragged."
		}
	case *browserproto.ScrollInput:
		text = scrollSummary(input, target)
	case *browserproto.FillInput:
		text = on("Filled")
	case *browserproto.TypeInput:
		if target != "" {
			text = "Typed into " + target + "."
		} else {
			text = "Typed."
		}
	case *browserproto.KeyInput:
		text = keySummary(input, target)
	case *browserproto.CheckInput:
		if input.Value {
			text = on("Checked")
		} else {
			text = on("Unchecked")
		}
	case *browserproto.SelectInput:
		var err error
		text, err = selectedOptionsText(result)
		if err != nil {
			return "", err
		}
	case *browserproto.SelectTextInput:
		text = selectedTextSummary(result, target)
	default:
		text = plain(result.text("operation")) + " completed."
	}

	return text, nil
}

func clickSummary(input *browserproto.ClickInput, target string, on func(string) string) string {
	text := ""
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

	return text
}

func renderActionOutcomeText(input browserproto.Input, object textObject) (string, error) {
	switch input.OperationName() {
	case "upload":
		return renderUploadText(input, object)
	case "download":
		return renderDownloadText(object)
	case "clipboard.write":
		return renderClipboardWriteText(object)
	case "clipboard.read":
		return renderClipboardReadText(object)
	case "eval":
		return renderEvalText(object)
	case "logs":
		return renderLogsText(object)
	case "viewport.set", "viewport.reset":
		return renderViewportSetText(object)
	case "dialog.inspect":
		return renderDialogInspectText(object)
	case "dialog.accept", "dialog.dismiss":
		return renderDialogAcceptText(object)
	default:
		return renderExtendedText(input, object)
	}
}

func scrollSummary(input *browserproto.ScrollInput, target string) string {
	text := ""
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

	return text
}

func keySummary(input *browserproto.KeyInput, target string) string {
	text := ""
	if target != "" {
		text = "Pressed " + plain(input.Key) + " in " + target + "."
	} else {
		text = "Pressed " + plain(input.Key) + "."
	}

	return text
}

func selectedTextSummary(result textObject, target string) string {
	text := ""
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
	return text
}
