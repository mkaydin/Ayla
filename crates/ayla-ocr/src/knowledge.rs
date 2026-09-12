use crate::output::PageResult;
use crate::types::{OcrKnowledgeEntity, OcrKnowledgePackage, OcrKnowledgeRelation, OcrPageResult};
use serde_json::{json, Map};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

const MAX_GRAPH_ENTITIES: usize = 20_000;
const MAX_ITEMS_PER_PAGE: usize = 64;
const MAX_NAME_BYTES: usize = 512;
const MAX_DESCRIPTION_BYTES: usize = 1_024;

pub(crate) fn build_knowledge_package(
    pages: &[PageResult],
    page_diagnostics: &[OcrPageResult],
    source_name: &str,
) -> OcrKnowledgePackage {
    let fingerprint = pages.iter().fold(Sha256::new(), |mut digest, page| {
        digest.update(page.page_num.to_le_bytes());
        digest.update(page.text.as_bytes());
        digest
    });
    let fingerprint = format!("{:x}", fingerprint.finalize());
    let document_uid = format!("ocr-document:{}", &fingerprint[..24]);
    let diagnostics = page_diagnostics
        .iter()
        .map(|page| (page.page, page))
        .collect::<HashMap<_, _>>();
    let mut entities = vec![OcrKnowledgeEntity {
        uid: document_uid.clone(),
        label: "Document".into(),
        name: bounded(source_name, MAX_NAME_BYTES),
        description: format!(
            "OCR document containing {} recognized page{}.",
            pages.len(),
            if pages.len() == 1 { "" } else { "s" }
        ),
        source_location: source_name.into(),
        confidence: 0.98,
        properties: Map::from_iter([
            ("recognized_pages".into(), json!(pages.len())),
            ("format".into(), json!("pdf")),
        ]),
    }];
    let mut relations = Vec::new();
    let mut warnings = Vec::new();

    for page in pages {
        if entities.len() >= MAX_GRAPH_ENTITIES {
            warnings.push(format!(
                "Structured graph preview was limited to {MAX_GRAPH_ENTITIES} entities"
            ));
            break;
        }
        let location = format!("{source_name}#page={}", page.page_num);
        let page_uid = format!("{document_uid}:page:{}", page.page_num);
        let diagnostic = diagnostics.get(&page.page_num).copied();
        let mut properties = Map::from_iter([
            ("page".into(), json!(page.page_num)),
            ("characters".into(), json!(page.text.chars().count())),
        ]);
        if let Some(diagnostic) = diagnostic {
            properties.insert("columns".into(), json!(diagnostic.columns));
            properties.insert("layout_source".into(), json!(diagnostic.layout_source));
        }
        entities.push(OcrKnowledgeEntity {
            uid: page_uid.clone(),
            label: "DocumentSection".into(),
            name: format!("{} — Page {}", bounded(source_name, 400), page.page_num),
            description: bounded(&plain_preview(&page.text), MAX_DESCRIPTION_BYTES),
            source_location: location.clone(),
            confidence: 0.95,
            properties,
        });
        relations.push(relation(
            &page_uid,
            "PART_OF",
            &document_uid,
            &location,
            1.0,
        ));

        if let Some(diagnostic) = diagnostic {
            for (index, heading) in diagnostic
                .headings
                .iter()
                .filter(|heading| !heading.trim().is_empty())
                .take(MAX_ITEMS_PER_PAGE)
                .enumerate()
            {
                if entities.len() >= MAX_GRAPH_ENTITIES {
                    break;
                }
                let name = bounded(heading.trim(), MAX_NAME_BYTES);
                let uid = stable_uid(
                    "ocr-heading",
                    &format!("{document_uid}|{}|{index}|{name}", page.page_num),
                );
                entities.push(OcrKnowledgeEntity {
                    uid: uid.clone(),
                    label: "SectionHeading".into(),
                    name,
                    description: format!("Heading recognized on page {}.", page.page_num),
                    source_location: location.clone(),
                    confidence: 0.9,
                    properties: Map::from_iter([("page".into(), json!(page.page_num))]),
                });
                relations.push(relation(&uid, "APPEARS_ON", &page_uid, &location, 0.95));
            }
            for (index, figure) in diagnostic
                .figures
                .iter()
                .take(MAX_ITEMS_PER_PAGE)
                .enumerate()
            {
                if entities.len() >= MAX_GRAPH_ENTITIES {
                    break;
                }
                let caption = if figure.caption.trim().is_empty() {
                    format!("Figure {} on page {}", index + 1, page.page_num)
                } else {
                    bounded(figure.caption.trim(), MAX_NAME_BYTES)
                };
                let uid = stable_uid(
                    "ocr-figure",
                    &format!("{document_uid}|{}|{index}|{caption}", page.page_num),
                );
                entities.push(OcrKnowledgeEntity {
                    uid: uid.clone(),
                    label: "Figure".into(),
                    name: caption.clone(),
                    description: format!("Figure evidence recognized on page {}.", page.page_num),
                    source_location: location.clone(),
                    confidence: 0.85,
                    properties: Map::from_iter([
                        ("page".into(), json!(page.page_num)),
                        ("caption".into(), json!(caption)),
                        (
                            "bounds".into(),
                            json!([figure.x0, figure.y0, figure.x1, figure.y1]),
                        ),
                    ]),
                });
                relations.push(relation(&uid, "APPEARS_ON", &page_uid, &location, 0.9));
            }
        }
    }

    if entities.len() >= MAX_GRAPH_ENTITIES
        && !warnings
            .iter()
            .any(|warning| warning.contains("Structured graph preview"))
    {
        warnings.push(format!(
            "Structured graph preview was limited to {MAX_GRAPH_ENTITIES} entities"
        ));
    }

    let knowledge_base_markdown = render_knowledge_markdown(
        pages,
        page_diagnostics,
        source_name,
        &document_uid,
        &entities,
        &relations,
        &warnings,
    );
    OcrKnowledgePackage {
        schema_version: 1,
        source_name: source_name.into(),
        document_uid,
        knowledge_base_markdown,
        entities,
        relations,
        warnings,
    }
}

pub(crate) fn build_evidence_package(
    pages: &[PageResult],
    page_diagnostics: &[OcrPageResult],
    source_name: &str,
) -> OcrKnowledgePackage {
    let mut package = build_knowledge_package(pages, page_diagnostics, source_name);
    package.entities.clear();
    package.relations.clear();
    package.knowledge_base_markdown = render_knowledge_markdown(
        pages,
        page_diagnostics,
        source_name,
        &package.document_uid,
        &package.entities,
        &package.relations,
        &package.warnings,
    )
    .replace(
        "Extraction: deterministic local OCR",
        "Extraction: local OCR; structured KB graph disabled",
    );
    package
}

fn render_knowledge_markdown(
    pages: &[PageResult],
    diagnostics: &[OcrPageResult],
    source_name: &str,
    document_uid: &str,
    entities: &[OcrKnowledgeEntity],
    relations: &[OcrKnowledgeRelation],
    warnings: &[String],
) -> String {
    let mut markdown = format!(
        "# Knowledge Base Document: {}\n\n## Provenance\n\n- Source: `{}`\n- Document UID: `{}`\n- Extraction: deterministic local OCR\n- Recognized pages: {}\n- Evidence policy: page-cited OCR text is preserved below\n\n",
        markdown_inline(source_name),
        markdown_inline(source_name),
        document_uid,
        pages.len()
    );
    markdown
        .push_str("## Knowledge Units\n\n| UID | Type | Name | Citation |\n|---|---|---|---|\n");
    for entity in entities {
        markdown.push_str(&format!(
            "| `{}` | {} | {} | `{}` |\n",
            entity.uid,
            markdown_table(&entity.label),
            markdown_table(&entity.name),
            markdown_table(&entity.source_location),
        ));
    }
    markdown.push_str("\n## Relationships\n\n| Source UID | Relation | Target UID | Citation |\n|---|---|---|---|\n");
    for relation in relations {
        markdown.push_str(&format!(
            "| `{}` | `{}` | `{}` | `{}` |\n",
            relation.source,
            relation.relation,
            relation.target,
            markdown_table(&relation.source_location),
        ));
    }
    if !warnings.is_empty() {
        markdown.push_str("\n## Preparation Warnings\n\n");
        for warning in warnings {
            markdown.push_str(&format!("- {}\n", markdown_inline(warning)));
        }
    }
    let diagnostic_by_page = diagnostics
        .iter()
        .map(|page| (page.page, page))
        .collect::<HashMap<_, _>>();
    markdown.push_str("\n## Page-Cited Evidence\n\n");
    for page in pages {
        markdown.push_str(&format!("### Page {}\n\n", page.page_num));
        if let Some(diagnostic) = diagnostic_by_page.get(&page.page_num) {
            if !diagnostic.headings.is_empty() {
                markdown.push_str(&format!(
                    "Recognized headings: {}\n\n",
                    diagnostic
                        .headings
                        .iter()
                        .map(|heading| format!("`{}`", markdown_inline(heading)))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        markdown.push_str(page.text.trim());
        markdown.push_str("\n\n---\n\n");
    }
    markdown
}

fn relation(
    source: &str,
    relation: &str,
    target: &str,
    location: &str,
    confidence: f32,
) -> OcrKnowledgeRelation {
    OcrKnowledgeRelation {
        source: source.into(),
        relation: relation.into(),
        target: target.into(),
        source_location: location.into(),
        confidence,
        properties: Map::new(),
    }
}

fn stable_uid(namespace: &str, value: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(value.as_bytes()));
    format!("{namespace}:{}", &digest[..24])
}

fn plain_preview(value: &str) -> String {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .take(8)
        .collect::<Vec<_>>()
        .join(" ")
}

fn bounded(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", value[..end].trim_end())
}

fn markdown_inline(value: &str) -> String {
    value.replace(['\r', '\n'], " ").replace('`', "'")
}

fn markdown_table(value: &str) -> String {
    markdown_inline(value).replace('|', "\\|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FigureAsset, LayoutSource};

    #[test]
    fn knowledge_package_is_stable_cited_and_graph_ready() {
        let pages = vec![PageResult {
            page_num: 1,
            text: "# Results\nMeasured value: 42.75 MPa.".into(),
            image_path: String::new(),
        }];
        let diagnostics = vec![OcrPageResult {
            page: 1,
            chars: 35,
            columns: 1,
            headings: vec!["Results".into()],
            figures: vec![FigureAsset {
                x0: 1.0,
                y0: 2.0,
                x1: 3.0,
                y1: 4.0,
                caption: "Figure 1: Strength".into(),
            }],
            layout_source: LayoutSource::PdfText,
            warnings: vec![],
            cached: false,
        }];
        let first = build_knowledge_package(&pages, &diagnostics, "fixture.pdf");
        let second = build_knowledge_package(&pages, &diagnostics, "renamed.pdf");
        assert_eq!(first.document_uid, second.document_uid);
        assert_eq!(first.entities.len(), 4);
        assert_eq!(first.relations.len(), 3);
        assert!(first.knowledge_base_markdown.contains("42.75 MPa"));
        assert!(first.knowledge_base_markdown.contains("fixture.pdf#page=1"));
        assert!(first.relations.iter().all(|relation| first
            .entities
            .iter()
            .any(|entity| entity.uid == relation.source)
            && first
                .entities
                .iter()
                .any(|entity| entity.uid == relation.target)));
    }

    #[test]
    fn evidence_mode_preserves_pages_without_structured_graph_records() {
        let pages = vec![PageResult {
            page_num: 1,
            text: "Evidence remains intact.".into(),
            image_path: String::new(),
        }];
        let package = build_evidence_package(&pages, &[], "fixture.pdf");
        assert!(package.entities.is_empty());
        assert!(package.relations.is_empty());
        assert!(package
            .knowledge_base_markdown
            .contains("Evidence remains intact."));
        assert!(package
            .knowledge_base_markdown
            .contains("structured KB graph disabled"));
    }
}
