//! Visual parity: renders a deck with LibreOffice and compares it, pixel by
//! pixel, with the CPU renderer. LibreOffice is the reference viewer that
//! runs without a person; it does not draw everything as PowerPoint does.

use std::path::{Path, PathBuf};
use std::process::Command;

use tiny_skia::Pixmap;

/// Pixels of the slide width in one inch, at scale 1: a 1600 unit slide is
/// 13.333 inches.
const DPI_PER_SCALE: f32 = 120.;

/// A channel difference more than this makes a pixel different. Edges
/// that anti-alias a little differently stay below it.
pub const PIXEL_TOLERANCE: u8 = 48;

/// True when LibreOffice and pdftoppm are on the PATH.
pub fn available() -> bool {
    ["soffice", "pdftoppm"].iter().all(|tool| {
        Command::new("which")
            .arg(tool)
            .output()
            .is_ok_and(|output| output.status.success())
    })
}

/// Renders each slide of `pptx` to a pixmap, with LibreOffice (to PDF) and
/// pdftoppm. `work` holds the files made on the way.
pub fn render(pptx: &Path, work: &Path, scale: f32) -> Result<Vec<Pixmap>, String> {
    std::fs::create_dir_all(work).map_err(|error| error.to_string())?;
    // A profile of its own, so a LibreOffice that is open does not take
    // the conversion.
    let profile = work.join("profile");
    let status = Command::new("soffice")
        .arg(format!(
            "-env:UserInstallation=file://{}",
            absolute(&profile).display()
        ))
        .args(["--headless", "--convert-to", "pdf", "--outdir"])
        .arg(work)
        .arg(pptx)
        .output()
        .map_err(|error| format!("cannot run soffice: {error}"))?;
    let stem = pptx
        .file_stem()
        .ok_or("the PPTX path has no file name")?
        .to_string_lossy()
        .to_string();
    let pdf = work.join(format!("{stem}.pdf"));
    if !pdf.exists() {
        return Err(format!(
            "soffice made no PDF: {}",
            String::from_utf8_lossy(&status.stderr)
        ));
    }
    let prefix = work.join(&stem);
    let dpi = (DPI_PER_SCALE * scale).round().max(1.);
    let output = Command::new("pdftoppm")
        .args(["-png", "-r"])
        .arg(dpi.to_string())
        .arg(&pdf)
        .arg(&prefix)
        .output()
        .map_err(|error| format!("cannot run pdftoppm: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "pdftoppm failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let mut pages: Vec<PathBuf> = std::fs::read_dir(work)
        .map_err(|error| error.to_string())?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == "png")
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(&format!("{stem}-")))
        })
        .collect();
    // pdftoppm numbers the pages with a fixed width: the names sort.
    pages.sort();
    pages
        .iter()
        .map(|page| Pixmap::load_png(page).map_err(|error| error.to_string()))
        .collect()
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// How two renders of a slide differ.
pub struct Difference {
    /// Mean of the channel differences, 0 to 255.
    pub mean: f32,
    /// Fraction of the pixels with a channel difference above
    /// [`PIXEL_TOLERANCE`].
    pub differing: f32,
    /// The differing pixels in red on a faded copy of the reference.
    pub image: Pixmap,
}

/// Compares `other` with `reference`. When the sizes differ by a pixel
/// or two (rounding of the page size), the common area is compared.
pub fn difference(reference: &Pixmap, other: &Pixmap) -> Difference {
    let width = reference.width().min(other.width());
    let height = reference.height().min(other.height());
    let mut image = Pixmap::new(width.max(1), height.max(1)).expect("non-zero size");
    let mut total = 0u64;
    let mut differing = 0u64;
    let reference_data = reference.data();
    let other_data = other.data();
    let out = image.data_mut();
    for y in 0..height {
        for x in 0..width {
            let a = ((y * reference.width() + x) * 4) as usize;
            let b = ((y * other.width() + x) * 4) as usize;
            let o = ((y * width + x) * 4) as usize;
            let mut worst = 0u8;
            for channel in 0..3 {
                let delta = reference_data[a + channel].abs_diff(other_data[b + channel]);
                total += delta as u64;
                worst = worst.max(delta);
            }
            if worst > PIXEL_TOLERANCE {
                differing += 1;
                out[o..o + 4].copy_from_slice(&[255, 0, 0, 255]);
            } else {
                // A light copy of the reference, to see where things are.
                for channel in 0..3 {
                    out[o + channel] = 255 - (255 - reference_data[a + channel]) / 4;
                }
                out[o + 3] = 255;
            }
        }
    }
    let pixels = (width as u64 * height as u64).max(1);
    Difference {
        mean: total as f32 / (pixels * 3) as f32,
        differing: differing as f32 / pixels as f32,
        image,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(width: u32, height: u32, color: tiny_skia::Color) -> Pixmap {
        let mut pixmap = Pixmap::new(width, height).unwrap();
        pixmap.fill(color);
        pixmap
    }

    #[test]
    fn equal_images_do_not_differ() {
        let image = filled(4, 3, tiny_skia::Color::WHITE);
        let difference = difference(&image, &image);
        assert_eq!(difference.mean, 0.);
        assert_eq!(difference.differing, 0.);
    }

    #[test]
    fn counts_the_pixels_past_the_tolerance() {
        let white = filled(2, 2, tiny_skia::Color::WHITE);
        let mut other = white.clone();
        other.data_mut()[0] = 0;
        let difference = difference(&white, &other);
        assert_eq!(difference.differing, 0.25);
        assert_eq!(difference.image.pixel(0, 0).unwrap().red(), 255);
        assert_eq!(difference.image.pixel(0, 0).unwrap().green(), 0);
    }
}
