use crate::model::{
    code_embedding_text, short_hash, Entity, GenerateError, KnowledgePackage, Relation,
};
use tree_sitter::{Node, Parser};

pub(crate) fn analyze_snippet(name: &str, source: &str) -> Result<KnowledgePackage, GenerateError> {
    let mut package = KnowledgePackage::new(name.to_owned(), &short_hash(source));
    analyze_file(&mut package, "snippet.rs", source)?;
    Ok(package)
}

pub(crate) fn analyze_file(
    package: &mut KnowledgePackage,
    file: &str,
    source: &str,
) -> Result<(), GenerateError> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
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
        embedding_text: Some(format!("Rust source file {file}.")),
        properties: Default::default(),
        source_excerpt: Some(source.to_owned()),
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
    impl_type: Option<&str>,
) {
    let kind = node.kind();
    if let Some((label, name)) = declaration(node, source, impl_type) {
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
        let next_impl = if kind == "impl_item" {
            type_name(node, source).or(impl_type.map(str::to_owned))
        } else {
            impl_type.map(str::to_owned)
        };
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            visit(package, child, source, file, &uid, next_impl.as_deref());
        }
        return;
    }
    if kind == "use_declaration" {
        let text = node
            .utf8_text(source.as_bytes())
            .unwrap_or("")
            .trim()
            .trim_start_matches("use ")
            .trim_end_matches(';')
            .to_owned();
        if !text.is_empty() {
            let dependency_uid = format!("{}:dependency:{}", package.repository_uid, text);
            package.add_entity(Entity {
                uid: dependency_uid.clone(),
                label: "Dependency".into(),
                name: text,
                description: Some("Rust import discovered in source.".into()),
                source_location: Some(format!("{}:{}", file, node.start_position().row + 1)),
                origin: "AST".into(),
                confidence: 1.0,
                embedding_text: None,
                properties: Default::default(),
                source_excerpt: None,
            });
            package.add_relation(relation(parent_uid, "IMPORTS", &dependency_uid, file));
        }
    }
    let next_impl = if kind == "impl_item" {
        type_name(node, source).or(impl_type.map(str::to_owned))
    } else {
        impl_type.map(str::to_owned)
    };
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit(
            package,
            child,
            source,
            file,
            parent_uid,
            next_impl.as_deref(),
        );
    }
}

fn declaration(
    node: Node,
    source: &str,
    impl_type: Option<&str>,
) -> Option<(&'static str, String)> {
    let label = match node.kind() {
        "struct_item" => "Struct",
        "enum_item" => "Enum",
        "trait_item" => "Trait",
        "mod_item" => "Module",
        "function_item" if impl_type.is_some() => "Method",
        "function_item" => "Function",
        _ => return None,
    };
    let name = node
        .child_by_field_name("name")?
        .utf8_text(source.as_bytes())
        .ok()?
        .to_owned();
    Some((
        label,
        match impl_type {
            Some(ty) if label == "Method" => format!("{ty}::{name}"),
            _ => name,
        },
    ))
}

fn type_name(node: Node, source: &str) -> Option<String> {
    node.child_by_field_name("type")?
        .utf8_text(source.as_bytes())
        .ok()
        .map(str::to_owned)
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
