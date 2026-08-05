pub struct PageResult {
    pub page_num: u16,
    pub text: String,
    pub image_path: String,
}

pub fn build_markdown(
    pages: &[PageResult],
    source_pdf: &str,
    prompt: &str,
    model: &str,
    include_images: bool,
) -> String {
    let mut md = String::new();

    md.push_str(&format!("# OCR Output: {source_pdf}\n\n"));
    md.push_str(&format!(
        "*Generated with `{model}` | {} pages | {}*\n",
        pages.len(),
        chrono::Local::now().format("%Y-%m-%d %H:%M")
    ));
    md.push_str(&format!("*Prompt: \"{prompt}\"*\n\n"));
    md.push_str("---\n\n");

    for page in pages {
        md.push_str(&format!("## Page {}\n\n", page.page_num));
        if include_images {
            md.push_str(&format!(
                "![Page {}]({})\n\n",
                page.page_num, page.image_path
            ));
        }
        if page.text.is_empty() {
            md.push_str("*(no text extracted)*\n");
        } else {
            md.push_str(&page.text);
        }
        md.push_str("\n\n---\n\n");
    }

    md
}
