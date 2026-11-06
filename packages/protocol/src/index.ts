// The TypeScript form of the Rust contracts the web app and a program's SDK
// read: the core's types and frames, the web API's request and response
// bodies, and a program's plugins' contract.
//
// `bun run contracts` generates the schemas, types and tables into
// ./generated from the Rust types. Nothing generated is committed, and the
// package imports no other workspace package.
//
// Beside them, what every client does with them alike: `schemaRefusal` words
// a value the schemas refused, and `withPart` applies a part of the product
// state a message or a write's answer carries.
export * from './generated/contracts'
export * from './generated/tables'
export * from './generated/web-api'
export * from './product-state'
export * from './refusal'
