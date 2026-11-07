//! Message formatting for Mtrx, shared by all frontends:
//!
//! - [`markdown_to_matrix`]: composer text (Markdown, `$…$` math) to Matrix
//!   message bodies, compatible with Element, with mentions as
//!   `matrix.to` links.
//! - [`Document`]: received Matrix HTML as a small document model; math keeps
//!   its LaTeX source. Copied, all or in part, as HTML for other apps.
//! - [`TextBlock`]: a document flattened for native text views, with math
//!   as [`Formula`]s.
//! - [`Renderer`] (feature `render`): typeset a document, math included, into
//!   an image with Typst; or one formula. Fonts: all of Typst's (feature
//!   `typst-fonts`) or only the math font (`math-font`).

mod copy;
mod document;
mod markdown;
#[cfg(feature = "render")]
mod render;
mod text;

pub use document::{Block, Document, Inline, Style};
pub use markdown::{Formatted, Mention, MentionTarget, find_mentions, markdown_to_matrix};
#[cfg(feature = "render")]
pub use render::{FormulaImage, Image, Piece, RenderError, RenderStyle, Renderer};
pub use text::{BlockKind, Formula, Run, TextBlock, TextStyle};
