//! Message formatting for Mtrx, shared by all frontends:
//!
//! - [`markdown_to_matrix`]: composer text (Markdown, `$…$` math) to Matrix
//!   message bodies, compatible with Element, with mentions as
//!   `matrix.to` links.
//! - [`Document`]: received Matrix HTML as a small document model; math keeps
//!   its LaTeX source. Copied, all or in part, as HTML for other apps.
//! - [`Renderer`] (feature `render`): typeset a document, math included, into
//!   an image with Typst.

mod copy;
mod document;
mod markdown;
#[cfg(feature = "render")]
mod render;

pub use document::{Block, Document, Inline, Style};
pub use markdown::{Formatted, Mention, MentionTarget, escape, find_mentions, markdown_to_matrix};
#[cfg(feature = "render")]
pub use render::{Image, Piece, RenderError, RenderStyle, Renderer};
