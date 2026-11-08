//! The core of Holm: notes, files, links, the index and file watching.
//!
//! It has no UI. The app and the later manager use it through one `Core` handle.

/// An error from `holm-core`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {}
