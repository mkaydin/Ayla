use crate::{
    javascript_parser,
    model::{short_hash, GenerateError, KnowledgePackage},
};

pub(crate) fn analyze_snippet(name: &str, source: &str) -> Result<KnowledgePackage, GenerateError> {
    let mut package = KnowledgePackage::new(name.to_owned(), &short_hash(source));
    analyze_file(&mut package, "snippet.ts", source)?;
    Ok(package)
}

pub(crate) fn analyze_file(
    package: &mut KnowledgePackage,
    file: &str,
    source: &str,
) -> Result<(), GenerateError> {
    let language = if file.ends_with(".tsx") {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    };
    javascript_parser::analyze_with_language(package, file, source, language)
}
