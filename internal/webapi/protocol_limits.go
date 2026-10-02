package webapi

// The largest message a page sends on any of its WebSockets: the
// conversation socket, the synchronization channel and a user stream
// (`web-api.md` § Request bodies). A frame refers to an upload and never
// carries its bytes, and a user stream frames its own messages, so no page
// needs a larger one.
// +demi:table
const MaxPageMessageBytes = 1024 * 1024
