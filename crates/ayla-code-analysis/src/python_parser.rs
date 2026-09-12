use tree_sitter::{Node, Parser};

use crate::model::{
    code_embedding_text, short_hash, Entity, GenerateError, KnowledgePackage, Relation,
};

pub(crate) fn analyze_snippet(name: &str, source: &str) -> Result<KnowledgePackage, GenerateError> {
    let mut package = KnowledgePackage::new(name.to_owned(), &short_hash(source));
    analyze_file(&mut package, "snippet.py", source)?;
    Ok(package)
}

pub(crate) fn analyze_file(
    package: &mut KnowledgePackage,
    file: &str,
    source: &str,
) -> Result<(), GenerateError> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .map_err(|error| GenerateError::Parse(error.to_string()))?;
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
        embedding_text: Some(format!("Python source file {file}.")),
        properties: Default::default(),
        source_excerpt: Some(source.chars().take(12_000).collect()),
    });
    package.add_relation(relation(
        &package.repository_uid,
        "CONTAINS",
        &file_uid,
        file,
    ));
    visit(package, tree.root_node(), source, file, &file_uid, None);
    Ok(())
}

fn visit(
    package: &mut KnowledgePackage,
    node: Node,
    source: &str,
    file: &str,
    parent_uid: &str,
    class_name: Option<&str>,
) {
    if let Some((label, name)) = declaration(node, source, class_name) {
        let uid = format!(
            "{}:{}:{}::{}",
            package.repository_uid,
            label.to_ascii_lowercase(),
            file,
            name
        );
        let location = format!(
            "{}:{}-{}",
            file,
            node.start_position().row + 1,
            node.end_position().row + 1
        );
        package.add_entity(Entity {
            uid: uid.clone(),
            label: label.into(),
            name: name.clone(),
            description: None,
            source_location: Some(location.clone()),
            origin: "AST".into(),
            confidence: 1.0,
            embedding_text: Some(code_embedding_text(
                label,
                &name,
                file,
                node.utf8_text(source.as_bytes()).unwrap_or(""),
            )),
            properties: Default::default(),
            source_excerpt: node
                .utf8_text(source.as_bytes())
                .ok()
                .map(|text| text.chars().take(12_000).collect()),
        });
        package.add_relation(relation(parent_uid, "DEFINES", &uid, &location));
        let next_class = if label == "Class" {
            Some(name)
        } else {
            class_name.map(str::to_owned)
        };
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            visit(package, child, source, file, &uid, next_class.as_deref());
        }
        return;
    }
    if matches!(node.kind(), "import_statement" | "import_from_statement") {
        let raw = node
            .utf8_text(source.as_bytes())
            .unwrap_or("")
            .trim()
            .to_owned();
        let name = normalized_import_package(&raw);
        if !name.is_empty() {
            let uid = format!("{}:dependency-package:{}", package.repository_uid, name);
            package.add_entity(Entity {
                uid: uid.clone(),
                label: "DependencyPackage".into(),
                name,
                description: Some(format!("Python import discovered: {raw}")),
                source_location: Some(format!("{}:{}", file, node.start_position().row + 1)),
                origin: "AST".into(),
                confidence: 1.0,
                embedding_text: None,
                properties: Default::default(),
                source_excerpt: None,
            });
            package.add_relation(relation(parent_uid, "IMPORTS", &uid, file));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit(package, child, source, file, parent_uid, class_name);
    }
}

fn normalized_import_package(raw: &str) -> String {
    let candidate = raw
        .strip_prefix("from ")
        .and_then(|value| value.split_whitespace().next())
        .or_else(|| {
            raw.strip_prefix("import ")
                .and_then(|value| value.split(',').next())
        })
        .unwrap_or(raw)
        .trim();
    candidate
        .split('.')
        .next()
        .unwrap_or(candidate)
        .trim()
        .to_owned()
}

fn declaration(
    node: Node,
    source: &str,
    class_name: Option<&str>,
) -> Option<(&'static str, String)> {
    let label = match node.kind() {
        "class_definition" => "Class",
        "function_definition" if class_name.is_some() => "Method",
        "function_definition" => "Function",
        _ => return None,
    };
    let name = node
        .child_by_field_name("name")?
        .utf8_text(source.as_bytes())
        .ok()?
        .to_owned();
    Some((
        label,
        match class_name {
            Some(class_name) if label == "Method" => format!("{class_name}::{name}"),
            _ => name,
        },
    ))
}

fn relation(source: &str, kind: &str, target: &str, location: &str) -> Relation {
    Relation {
        source: source.into(),
        relation: kind.into(),
        target: target.into(),
        origin: "AST".into(),
        confidence: 1.0,
        source_location: Some(location.into()),
        properties: Default::default(),
    }
}
