// Package commandtree holds command declarations: the tree of groups and
// leaves that a command manifest carries, the rules a leaf's input declaration
// follows, and how a command line selects a command, fills its input and
// renders its help (docs/execution/commands.md).
//
// A declaration is a wire type, so a manifest's nodes and the backend's
// declarations are one definition. A native leaf names its operation with a
// [Binding]; a declaration leaves the binding's descriptor hash empty, and
// [Pin] fills it from the package descriptors a manifest carries.
//
// The package does no IO: it neither reads stdin nor holds handlers.
package commandtree
