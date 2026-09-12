use crate::types::{FigureAsset, LayoutSource, PageLayout};

#[derive(Debug, Clone, PartialEq)]
pub struct TextSpan {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub text: String,
}

const COLUMN_GAP_FRAC: f32 = 0.18;
const COLUMN_MIN_LINES: usize = 3;
const HEADING_HEIGHT_MULT: f32 = 1.8;
const CAPTION_MAX_DIST: f32 = 70.0;

pub fn analyze_page(page: u16, width: f32, height: f32, spans: Vec<TextSpan>) -> PageLayout {
    let mut lines = spans
        .into_iter()
        .filter(|span| !span.text.trim().is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return PageLayout::unknown(page, width, height);
    }
    lines.sort_by(|left, right| {
        left.y0
            .total_cmp(&right.y0)
            .then_with(|| left.x0.total_cmp(&right.x0))
    });
    let body = lines
        .iter()
        .filter(|line| line.text.chars().count() >= 8)
        .collect::<Vec<_>>();
    let columns = detect_columns(&body, width);
    let headings = detect_headings(&body, height, width);
    let figures = caption_assets(&lines, width, height);
    PageLayout {
        page,
        width_pt: width,
        height_pt: height,
        columns,
        headings,
        figures,
        source: LayoutSource::PdfText,
    }
}

fn detect_columns(lines: &[&TextSpan], width: f32) -> u8 {
    if lines.len() < 8 || width <= 0.0 {
        return 1;
    }
    let centers = lines
        .iter()
        .map(|line| (line.x0 + line.x1) / 2.0)
        .collect::<Vec<_>>();
    let mut left = centers.iter().copied().fold(f32::INFINITY, f32::min);
    let mut right = centers.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    for _ in 0..30 {
        let (mut left_sum, mut left_count, mut right_sum, mut right_count) =
            (0.0, 0usize, 0.0, 0usize);
        for center in &centers {
            if (center - left).abs() <= (center - right).abs() {
                left_sum += center;
                left_count += 1;
            } else {
                right_sum += center;
                right_count += 1;
            }
        }
        let next_left = if left_count == 0 {
            left
        } else {
            left_sum / left_count as f32
        };
        let next_right = if right_count == 0 {
            right
        } else {
            right_sum / right_count as f32
        };
        if (next_left - left).abs() < 0.5 && (next_right - right).abs() < 0.5 {
            break;
        }
        left = next_left;
        right = next_right;
    }
    let (low, high) = if left < right {
        (left, right)
    } else {
        (right, left)
    };
    let low_count = centers
        .iter()
        .filter(|center| (**center - low).abs() <= (**center - high).abs())
        .count();
    let high_count = centers.len() - low_count;
    if high - low > width * COLUMN_GAP_FRAC
        && low_count >= COLUMN_MIN_LINES
        && high_count >= COLUMN_MIN_LINES
    {
        2
    } else {
        1
    }
}

fn detect_headings(lines: &[&TextSpan], page_height: f32, page_width: f32) -> Vec<String> {
    let mut heights = lines
        .iter()
        .filter(|line| line.y0 > page_height * 0.06 && line.y1 < page_height * 0.94)
        .map(|line| line.y1 - line.y0)
        .filter(|height| *height > 0.0)
        .collect::<Vec<_>>();
    if heights.is_empty() {
        return Vec::new();
    }
    heights.sort_by(|left, right| left.total_cmp(right));
    let median = heights[heights.len() / 2].max(1.0);
    lines
        .iter()
        .filter(|line| {
            line.y1 - line.y0 > median * HEADING_HEIGHT_MULT
                && line.x1 - line.x0 < page_width * 0.9
                && line.text.chars().count() <= 200
                && line.x0 < page_width * 0.35
        })
        .map(|line| line.text.trim().to_string())
        .collect()
}

// PDF text geometry cannot reliably recover image object boundaries across all PDFs.
// Caption spans are still useful layout evidence, so expose conservative zero-area
// anchors instead of inventing figure rectangles from page whitespace.
fn caption_assets(lines: &[TextSpan], width: f32, height: f32) -> Vec<FigureAsset> {
    lines
        .iter()
        .filter(|line| is_caption(&line.text))
        .filter(|line| line.y0 >= 0.0 && line.y0 <= height + CAPTION_MAX_DIST)
        .take(20)
        .map(|line| FigureAsset {
            x0: line.x0.clamp(0.0, width),
            y0: line.y0.clamp(0.0, height),
            x1: line.x1.clamp(0.0, width),
            y1: line.y1.clamp(0.0, height),
            caption: line.text.trim().to_string(),
        })
        .collect()
}

fn is_caption(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    lower.starts_with("figure")
        || lower.starts_with("fig.")
        || lower.starts_with("fig ")
        || lower.starts_with("table")
        || lower.starts_with("abbildung")
        || lower.starts_with("tab.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(x0: f32, x1: f32, y: f32, text: impl Into<String>) -> TextSpan {
        TextSpan {
            x0,
            y0: y,
            x1,
            y1: y + 12.0,
            text: text.into(),
        }
    }

    #[test]
    fn no_text_layer_is_explicit() {
        let layout = analyze_page(3, 612.0, 792.0, Vec::new());
        assert_eq!(layout.source, LayoutSource::None);
        assert_eq!(layout.columns, 1);
    }

    #[test]
    fn detects_two_columns() {
        let mut spans = Vec::new();
        for row in 0..10 {
            spans.push(span(40.0, 270.0, row as f32 * 20.0, "left column content"));
            spans.push(span(
                340.0,
                570.0,
                row as f32 * 20.0,
                "right column content",
            ));
        }
        assert_eq!(analyze_page(1, 612.0, 792.0, spans).columns, 2);
    }

    #[test]
    fn exposes_caption_evidence() {
        let layout = analyze_page(
            1,
            612.0,
            792.0,
            vec![span(50.0, 400.0, 400.0, "Figure 2: Experimental setup")],
        );
        assert_eq!(layout.figures.len(), 1);
        assert_eq!(layout.figures[0].caption, "Figure 2: Experimental setup");
    }
}
