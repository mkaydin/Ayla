use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OcrKnowledgeMode {
    Disabled,
    Deterministic,
    Semantic,
}

impl Default for OcrKnowledgeMode {
    fn default() -> Self {
        Self::Deterministic
    }
}

impl OcrKnowledgeMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "disabled" | "off" | "none" => Some(Self::Disabled),
            "deterministic" | "structured" => Some(Self::Deterministic),
            "semantic" | "local_semantic" => Some(Self::Semantic),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Deterministic => "deterministic",
            Self::Semantic => "semantic",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayoutSource {
    PdfText,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FigureAsset {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub caption: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PageLayout {
    pub page: u16,
    pub width_pt: f32,
    pub height_pt: f32,
    pub columns: u8,
    pub headings: Vec<String>,
    pub figures: Vec<FigureAsset>,
    pub source: LayoutSource,
}

impl PageLayout {
    pub fn unknown(page: u16, width_pt: f32, height_pt: f32) -> Self {
        Self {
            page,
            width_pt,
            height_pt,
            columns: 1,
            headings: Vec::new(),
            figures: Vec::new(),
            source: LayoutSource::None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OcrWarning {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u16>,
}

impl OcrWarning {
    pub(crate) fn page(page: u16, code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            page: Some(page),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrPageResult {
    pub page: u16,
    pub chars: usize,
    pub columns: u8,
    pub headings: Vec<String>,
    pub figures: Vec<FigureAsset>,
    pub layout_source: LayoutSource,
    pub warnings: Vec<OcrWarning>,
    #[serde(default)]
    pub cached: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct OcrPerformanceMetrics {
    pub elapsed_ms: u64,
    pub pages_from_cache: usize,
    pub pages_recognized: usize,
    pub peak_rendered_page_bytes: u64,
    pub recognized_characters: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OcrModels {
    #[serde(default)]
    pub revision: Option<String>,
    pub ocr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assembly: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrKnowledgeEntity {
    pub uid: String,
    pub label: String,
    pub name: String,
    pub description: String,
    pub source_location: String,
    pub confidence: f32,
    #[serde(default)]
    pub properties: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrKnowledgeRelation {
    pub source: String,
    pub relation: String,
    pub target: String,
    pub source_location: String,
    pub confidence: f32,
    #[serde(default)]
    pub properties: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrKnowledgePackage {
    pub schema_version: u8,
    pub source_name: String,
    pub document_uid: String,
    pub knowledge_base_markdown: String,
    pub entities: Vec<OcrKnowledgeEntity>,
    pub relations: Vec<OcrKnowledgeRelation>,
    #[serde(default)]
    pub warnings: Vec<String>,
}
