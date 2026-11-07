//! Formulas typeset for the apps' message text.

use crate::{Formula, MtrxError, runtime::run};

/// A typeset formula.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FormulaImage {
    /// PNG, black on transparent: draw it tinted with the text's colour.
    pub png: Vec<u8>,
    /// In pixels.
    pub width: u32,
    pub height: u32,
    /// Pixels from the top to the baseline it sits on (at most `height`).
    pub baseline: u32,
}

/// Typeset `formula` at `em_pixels` device pixels per em (the body text size
/// times the screen's scale), rounded to whole pixels: keep the image per
/// formula and size. [`MtrxError::Other`] for LaTeX it cannot read: show
/// the formula's `tex` as code instead.
#[uniffi::export]
pub async fn typeset_formula(formula: Formula, em_pixels: f32) -> Result<FormulaImage, MtrxError> {
    run(async move {
        let typeset = mtrx_core::typeset(formula.into(), em_pixels).await?;
        Ok(FormulaImage {
            png: typeset.png,
            width: typeset.width,
            height: typeset.height,
            baseline: typeset.baseline,
        })
    })
    .await
}
