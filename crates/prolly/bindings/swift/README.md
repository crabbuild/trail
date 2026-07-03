# Prolly Swift Binding

This package exposes the Rust `prolly-map` engine through UniFFI-generated
Swift sources. The public API is byte-first and uses `Data` for keys, values,
CIDs, node bytes, and envelopes.

The generated API includes single-key, multi-key, range, cursor-page, cursor-resumed diff, diff-page, and prefix proof generation,
store-independent proof verification, named-root manifest metadata listing, retained named-root GC, compact path-node byte export/import,
canonical proof bundle bytes, proof-bundle introspection/routing summaries, one-shot proof-bundle verification, HMAC-authenticated proof envelopes, and one-shot authenticated proof-bundle verification for portable inclusion and absence checks. It also exposes `parallelBatch`, `batchWithStats`, and
`appendBatchWithStats` for parallel mutation application plus route and write-count telemetry, along with prefix bounds, segment encoding/decoding, numeric key helpers, and boundary checks.

Build the Rust facade before running Swift examples from the source tree:

```sh
cargo build -p prolly-bindings
cd crates/prolly/bindings/swift
DYLD_LIBRARY_PATH="$PWD/../../../../target/debug" swift run prolly-basic-map
```

The package links against `libprolly_bindings` from
`../../../../target/debug` by default. Set `PROLLY_BINDINGS_LIBRARY_DIR` when
the native library is somewhere else.

Generated UniFFI sources are checked in under `Sources/Prolly` and
`Sources/prollyFFI` for offline builds. Compiled native libraries and SwiftPM
`.build` output are intentionally not checked in.
