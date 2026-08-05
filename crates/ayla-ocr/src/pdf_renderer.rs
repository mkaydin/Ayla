use anyhow::Context;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops;
use image::DynamicImage;
use image::{ExtendedColorType, ImageEncoder, RgbaImage};
use pdfium_bundled::pdfium_render::prelude::*;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
pub enum OutputFormat {
    Png,
    Jpeg { quality: u8 },
}

#[derive(Clone, Copy)]
pub struct RenderConfig {
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
}

pub fn render_pages(
    pdf_path: &Path,
    first_page: u16,
    last_page: u16,
    config: &RenderConfig,
    images_dir: &Path,
) -> anyhow::Result<Vec<RenderResult>> {
    std::fs::create_dir_all(images_dir)
        .with_context(|| format!("Failed to create image dir: {}", images_dir.display()))?;

    let pdfium = pdfium_bundled::bind_pdfium_silent()
        .context("Failed to bind PDFium library. Is the download working?")?;

    let doc = pdfium
        .load_pdf_from_file(pdf_path, None)
        .with_context(|| format!("Failed to open PDF: {}", pdf_path.display()))?;

    let page_count = doc.pages().len();

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

    for idx in start..=end {
        let page_num = (idx + 1) as u16;

        let page = doc
            .pages()
            .get(idx)
            .with_context(|| format!("Failed to get page {page_num}"))?;

        let target_width = ((page.width().value * config.dpi / 72.0) as i32).min(config.max_width);
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

        std::fs::write(&image_path, &encoded)
            .with_context(|| format!("Failed to save image {}", image_path.display()))?;

        results.push(RenderResult {
            image_path,
            page_num,
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
