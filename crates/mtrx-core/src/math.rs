//! Formulas typeset for the phones, which draw message text natively and
//! place each formula in it as an image (feature `math`).

use std::{io::Cursor, sync::LazyLock};

use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
use mtrx_format::{Formula, Renderer};

use crate::{Error, Result};

/// Made on first use: Typst's library and the math font.
static RENDERER: LazyLock<Renderer> = LazyLock::new(Renderer::for_formulas);

/// A formula as a PNG: black ink on transparent, to tint with the text's
/// colour.
#[derive(Clone, Debug, PartialEq)]
pub struct Typeset {
    /// Grey and alpha.
    pub png: Vec<u8>,
    /// In pixels.
    pub width: u32,
    pub height: u32,
    /// Pixels from the top to the baseline it sits on (at most `height`).
    pub baseline: u32,
}

/// `formula` at `em_pixels` device pixels per em (the text size times the
/// screen's scale), rounded to whole pixels. Typesets on a blocking thread;
/// nothing is cached here (the apps keep the images).
pub async fn typeset(formula: Formula, em_pixels: f32) -> Result<Typeset> {
    let em_pixels = em_pixels.round().max(1.0);
    tokio::task::spawn_blocking(move || {
        let image = RENDERER
            .formula(&formula, em_pixels)
            .map_err(|error| Error::Formula(error.to_string()))?;
        let pixels: Vec<u8> = image.alpha.iter().flat_map(|&alpha| [0, alpha]).collect();
        let mut png = Vec::new();
        PngEncoder::new(Cursor::new(&mut png))
            .write_image(&pixels, image.width, image.height, ExtendedColorType::La8)
            .map_err(|error| Error::Formula(error.to_string()))?;
        Ok(Typeset {
            png,
            width: image.width,
            height: image.height,
            baseline: image.baseline,
        })
    })
    .await
    .map_err(|error| Error::Formula(error.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inline(tex: &str) -> Formula {
        Formula {
            tex: tex.into(),
            display: false,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_png_of_the_stated_size_also_when_concurrent() {
        let (fraction, power) = tokio::join!(
            typeset(inline(r"\frac{a}{b}"), 34.4),
            typeset(inline("x^2"), 34.0),
        );
        for typeset in [fraction.unwrap(), power.unwrap()] {
            let decoded = image::load_from_memory(&typeset.png).unwrap();
            assert_eq!(
                (decoded.width(), decoded.height()),
                (typeset.width, typeset.height)
            );
            assert!(typeset.baseline <= typeset.height);
            assert!(decoded.to_luma_alpha8().pixels().any(|p| p.0[1] > 0), "ink");
        }
    }

    #[tokio::test]
    async fn bad_latex_fails() {
        let result = typeset(inline(r"\notacommand{x}"), 34.0).await;
        assert!(matches!(result, Err(Error::Formula(_))), "{result:?}");
    }
}
