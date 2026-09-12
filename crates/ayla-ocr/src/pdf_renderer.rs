use anyhow::Context;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops;
use image::DynamicImage;
use image::{ExtendedColorType, ImageEncoder, RgbaImage};
use pdfium_bundled::pdfium_render::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

use crate::geometry::{self, TextSpan};
use crate::types::PageLayout;

// pdfium-render promotes the first dynamic binding into process-global state.
// Keep rendering serialized and reuse that binding for every later OCR job.
static PDFIUM_RENDER_STATE: Mutex<bool> = Mutex::new(false);

#[derive(Clone, Copy)]
pub enum OutputFormat {
    Png,
    Jpeg { quality: u8 },
}

#[derive(Clone)]
pub struct RenderConfig {
    pub library_path: Option<PathBuf>,
    pub dpi: f32,
    pub max_width: i32,
    pub format: OutputFormat,
    pub grayscale: bool,
    pub sharpen: bool,
    pub contrast: bool,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            library_path: None,
            dpi: 200.0,
            max_width: 2400,
            format: OutputFormat::Png,
            grayscale: true,
            sharpen: true,
            contrast: false,
        }
    }
}

pub struct RenderResult {
    pub image_path: PathBuf,
    pub page_num: u16,
    pub layout: PageLayout,
}

#[cfg(test)]
pub fn render_pages(
    pdf_path: &Path,
    first_page: u16,
    last_page: u16,
    config: &RenderConfig,
    images_dir: &Path,
) -> anyhow::Result<Vec<RenderResult>> {
    render_pages_cancellable(
        pdf_path,
        first_page,
        last_page,
        config,
        images_dir,
        &AtomicBool::new(false),
    )
}

pub fn render_pages_cancellable(
    pdf_path: &Path,
    first_page: u16,
    last_page: u16,
    config: &RenderConfig,
    images_dir: &Path,
    cancelled: &AtomicBool,
) -> anyhow::Result<Vec<RenderResult>> {
    std::fs::create_dir_all(images_dir)
        .with_context(|| format!("Failed to create image dir: {}", images_dir.display()))?;

    let mut initialized = PDFIUM_RENDER_STATE
        .lock()
        .map_err(|_| anyhow::anyhow!("PDFium render lock was poisoned"))?;
    let pdfium = if *initialized {
        Pdfium::default()
    } else {
        let pdfium = match &config.library_path {
            Some(path) => pdfium_bundled::bind_pdfium_from_path(path)
                .context("The packaged PDFium library could not be loaded")?,
            None => pdfium_bundled::bind_pdfium_silent().context("PDFium is not available")?,
        };
        *initialized = true;
        pdfium
    };

    let doc = pdfium
        .load_pdf_from_file(pdf_path, None)
        .with_context(|| format!("Failed to open PDF: {}", pdf_path.display()))?;

    let page_count = doc.pages().len();
    if page_count == 0 || page_count > 5000 {
        anyhow::bail!("PDF must contain 1–5000 pages");
    }

    let start = (first_page.saturating_sub(1) as i32).max(0);
    let end = if last_page == 0 {
        page_count - 1
    } else {
        ((last_page.saturating_sub(1) as i32).min(page_count - 1)).max(start)
    };

    let mut results = Vec::with_capacity((end - start + 1) as usize);
    let ext = match config.format {
        OutputFormat::Png => "png",
        OutputFormat::Jpeg { .. } => "jpg",
    };

    let mut total_encoded = 0usize;
    for idx in start..=end {
        if cancelled.load(Ordering::Relaxed) {
            anyhow::bail!("PDF rendering cancelled");
        }
        let page_num = (idx + 1) as u16;

        let page = doc
            .pages()
            .get(idx)
            .with_context(|| format!("Failed to get page {page_num}"))?;

        let page_width = page.width().value;
        let page_height = page.height().value;
        if !page_width.is_finite()
            || !page_height.is_finite()
            || page_width <= 0.0
            || page_height <= 0.0
        {
            anyhow::bail!("Page {page_num} has invalid dimensions");
        }
        let layout = match page.text() {
            Ok(text) => {
                let spans = text
                    .segments()
                    .iter()
                    .filter_map(|segment| {
                        let bounds = segment.bounds();
                        let value = segment.text().replace(['\r', '\n'], " ");
                        (!value.trim().is_empty()).then_some(TextSpan {
                            x0: bounds.left().value,
                            y0: page_height - bounds.top().value,
                            x1: bounds.right().value,
                            y1: page_height - bounds.bottom().value,
                            text: value,
                        })
                    })
                    .collect();
                geometry::analyze_page(page_num, page_width, page_height, spans)
            }
            Err(_) => PageLayout::unknown(page_num, page_width, page_height),
        };

        let target_width = ((page_width * config.dpi / 72.0) as i32).min(config.max_width);
        let target_width = target_width
            .min(4096)
            .min((8192.0 * page_width / page_height).max(1.0) as i32)
            .max(1);
        // Bound the raster before PDFium allocates: 16 million RGBA pixels.
        let pixel_width = (16_000_000.0_f64 * page_width as f64 / page_height as f64)
            .sqrt()
            .floor() as i32;
        let target_width = target_width.min(pixel_width.max(1));
        let render_config = PdfRenderConfig::new().set_target_width(target_width);

        let bitmap = page
            .render_with_config(&render_config)
            .with_context(|| format!("Failed to render page {page_num}"))?;

        let (width, height) = (bitmap.width() as u32, bitmap.height() as u32);
        let raw = bitmap.as_raw_bytes();

        let img = RgbaImage::from_raw(width, height, raw.to_vec())
            .with_context(|| "Failed to create image from bitmap")?;

        let mut dyn_img = DynamicImage::ImageRgba8(img);

        if config.grayscale {
            dyn_img = DynamicImage::ImageLuma8(imageops::grayscale(&dyn_img));
        }

        if config.contrast {
            dyn_img = stretch_contrast(&dyn_img);
        }

        if config.sharpen {
            let kernel: [f32; 9] = [0.0, -0.5, 0.0, -0.5, 3.0, -0.5, 0.0, -0.5, 0.0];
            dyn_img = dyn_img.filter3x3(&kernel);
        }

        let image_path = images_dir.join(format!("page_{:03}.{}", page_num, ext));
        let mut encoded = Vec::new();

        match config.format {
            OutputFormat::Png => {
                let encoder = PngEncoder::new(&mut encoded);
                encoder
                    .write_image(
                        dyn_img.as_bytes(),
                        dyn_img.width(),
                        dyn_img.height(),
                        dyn_img.color().into(),
                    )
                    .with_context(|| format!("Failed to encode page {page_num} as PNG"))?;
            }
            OutputFormat::Jpeg { quality } => {
                let encoder = JpegEncoder::new_with_quality(&mut encoded, quality);
                let rgb = dyn_img.to_rgb8();
                encoder
                    .write_image(&rgb, rgb.width(), rgb.height(), ExtendedColorType::Rgb8)
                    .with_context(|| format!("Failed to encode page {page_num} as JPEG"))?;
            }
        }

        total_encoded = total_encoded.saturating_add(encoded.len());
        if total_encoded > 512 * 1024 * 1024 {
            anyhow::bail!("Rendered PDF exceeds the 512 MiB temporary-storage budget");
        }
        if cancelled.load(Ordering::Relaxed) {
            anyhow::bail!("PDF rendering cancelled");
        }
        std::fs::write(&image_path, &encoded)
            .with_context(|| format!("Failed to save image {}", image_path.display()))?;

        results.push(RenderResult {
            image_path,
            page_num,
            layout,
        });
    }

    Ok(results)
}

fn stretch_contrast(img: &DynamicImage) -> DynamicImage {
    let gray = img.to_luma8();
    let (mut min_val, mut max_val) = (255u8, 0u8);
    for pixel in gray.pixels() {
        let v = pixel[0];
        if v < min_val {
            min_val = v;
        }
        if v > max_val {
            max_val = v;
        }
    }
    if max_val <= min_val {
        return DynamicImage::ImageLuma8(gray);
    }
    let scale = 255.0 / (max_val - min_val) as f32;
    let enhanced: Vec<u8> = gray
        .pixels()
        .flat_map(|p| {
            let v = ((p[0].saturating_sub(min_val)) as f32 * scale).round() as u8;
            [v]
        })
        .collect();
    let enhanced_img =
        image::GrayImage::from_raw(gray.width(), gray.height(), enhanced).unwrap_or(gray);
    DynamicImage::ImageLuma8(enhanced_img)
}
