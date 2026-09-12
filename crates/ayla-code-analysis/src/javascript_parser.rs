use crate::model::{
    code_embedding_text, short_hash, Entity, GenerateError, KnowledgePackage, Relation,
};
use tree_sitter::{Node, Parser};

pub(crate) fn analyze_snippet(name: &str, source: &str) -> Result<KnowledgePackage, GenerateError> {
    let mut package = KnowledgePackage::new(name.to_owned(), &short_hash(source));
    analyze_file(&mut package, "snippet.js", source)?;
    Ok(package)
}
pub(crate) fn analyze_file(
    package: &mut KnowledgePackage,
    file: &str,
    source: &str,
) -> Result<(), GenerateError> {
    analyze_with_language(
        package,
        file,
        source,
        tree_sitter_javascript::LANGUAGE.into(),
    )
}

pub(crate) fn analyze_with_language(
    package: &mut KnowledgePackage,
    file: &str,
    source: &str,
    language: tree_sitter::Language,
) -> Result<(), GenerateError> {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|e| GenerateError::Parse(e.to_string()))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| GenerateError::Parse("Tree-sitter produced no tree".into()))?;
    let file_uid = format!("{}:file:{file}", package.repository_uid);
    package.add_entity(Entity {
        uid: file_uid.clone(),
        label: "File".into(),
        name: file.into(),
        description: None,
        source_location: Some(file.into()),
        origin: "AST".into(),
        confidence: 1.0,
        embedding_text: Some(format!("JavaScript or TypeScript source file {file}.")),
        properties: Default::default(),
        source_excerpt: Some(source.chars().take(12_000).collect()),
    });
    package.add_relation(rel(&package.repository_uid, "CONTAINS", &file_uid, file));
    visit(package, tree.root_node(), source, file, &file_uid, None);
    Ok(())
}
fn visit(p: &mut KnowledgePackage, n: Node, s: &str, f: &str, parent: &str, class: Option<&str>) {
    if let Some((label, name)) = decl(n, s, class) {
        let uid = format!(
            "{}:{}:{}::{}",
            p.repository_uid,
            label.to_ascii_lowercase(),
            f,
            name
        );
        let loc = format!(
            "{}:{}-{}",
            f,
            n.start_position().row + 1,
            n.end_position().row + 1
        );
        p.add_entity(Entity {
            uid: uid.clone(),
            label: label.into(),
            name: name.clone(),
            description: None,
            source_location: Some(loc.clone()),
            origin: "AST".into(),
            confidence: 1.0,
            embedding_text: Some(code_embedding_text(
                label,
                &name,
                f,
                n.utf8_text(s.as_bytes()).unwrap_or(""),
            )),
            properties: Default::default(),
            source_excerpt: n
                .utf8_text(s.as_bytes())
                .ok()
                .map(|x| x.chars().take(12_000).collect()),
        });
        p.add_relation(rel(parent, "DEFINES", &uid, &loc));
        let next = if label == "Class" {
            Some(name)
        } else {
            class.map(str::to_owned)
        };
        let mut c = n.walk();
        for child in n.children(&mut c) {
            visit(p, child, s, f, &uid, next.as_deref());
        }
        return;
    }
    if n.kind() == "import_statement" {
        let raw = n.utf8_text(s.as_bytes()).unwrap_or("").trim().to_owned();
        let pkg = raw
            .split(" from ")
            .last()
            .unwrap_or(&raw)
            .trim_matches(|c| c == ';' || c == '\'' || c == '"')
            .to_owned();
        if !pkg.is_empty() {
            let uid = format!("{}:dependency-package:{}", p.repository_uid, pkg);
            p.add_entity(Entity {
                uid: uid.clone(),
                label: "DependencyPackage".into(),
                name: pkg,
                description: Some(format!("JavaScript import: {raw}")),
                source_location: Some(format!("{}:{}", f, n.start_position().row + 1)),
                origin: "AST".into(),
                confidence: 1.0,
                embedding_text: None,
                properties: Default::default(),
                source_excerpt: None,
            });
            p.add_relation(rel(parent, "IMPORTS", &uid, f));
        }
    }
    let mut c = n.walk();
    for child in n.children(&mut c) {
        visit(p, child, s, f, parent, class);
    }
}
fn decl(n: Node, s: &str, class: Option<&str>) -> Option<(&'static str, String)> {
    let label = match n.kind() {
        "class_declaration" => "Class",
        "function_declaration" => "Function",
        "method_definition" if class.is_some() => "Method",
        _ => return None,
    };
    let name = n
        .child_by_field_name("name")?
        .utf8_text(s.as_bytes())
        .ok()?
        .to_owned();
    Some((
        label,
        if label == "Method" {
            format!("{}::{name}", class?)
        } else {
            name
        },
    ))
}
fn rel(source: &str, relation: &str, target: &str, loc: &str) -> Relation {
    Relation {
        source: source.into(),
        relation: relation.into(),
        target: target.into(),
        origin: "AST".into(),
        confidence: 1.0,
        source_location: Some(loc.into()),
        properties: Default::default(),
    }
}
