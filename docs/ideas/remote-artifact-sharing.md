# Remote Artifact Sharing

## Status

Deferred. Dotall v0 stores all artifacts in the project-local `.all/` directory.

## Motivation

Parsing, transcription, indexing, semantic analysis, and projection generation can
be expensive. A remote artifact store could let authorized users reuse these results
when they encounter the same file content, reducing repeated compute and agent cost.

## Direction

- Address reusable artifacts by source content hash, schema version, format module
  version, and derivation configuration.
- Keep the project-local `.all/` directory as the working cache and source of local
  state.
- Fetch immutable read artifacts from a remote store on a local cache miss.
- Upload artifacts only with explicit user or organization policy.
- Treat permissions, encryption, retention, provenance, and cache poisoning as
  first-class design concerns.
- Keep edit history and mutable project state local unless a separate synchronization
  design is approved.

## Non-goal for v0

Dotall v0 will not implement a global local cache, remote storage, cross-user
sharing, or edit-history synchronization.
