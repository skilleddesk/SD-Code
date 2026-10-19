//! Tree-sitter parsing for the Code Graph (P1-1): the languages SDC reads, what a file declares, where it refers to names, what it imports,
//! and - for the edit gate (P1-2, P1-11) - whether its syntax is broken.
//!
//! Everything here is a pure function of a file's text. There is no model, no network and no state; the index (`graph::index`) caches
//! the answers by content hash.

use sha2::{Digest, Sha256};
use tree_sitter::{Language, Node, Parser, Tree};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Go,
    Php,
    Json,
    Bash,
}

impl Lang {
    /// The grammar for a path, from its extension.
    pub fn of(path: &str) -> Option<Self> {
        let extension = path.rsplit('.').next()?.to_ascii_lowercase();

        match extension.as_str() {
            "rs" => Some(Self::Rust),
            "py" | "pyi" => Some(Self::Python),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "ts" | "mts" | "cts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            "go" => Some(Self::Go),
            "php" | "phtml" => Some(Self::Php),
            "json" => Some(Self::Json),
            "sh" | "bash" => Some(Self::Bash),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::Go => "go",
            Self::Php => "php",
            Self::Json => "json",
            Self::Bash => "bash",
        }
    }

    fn grammar(self) -> Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::Php => tree_sitter_php::LANGUAGE_PHP.into(),
            Self::Json => tree_sitter_json::LANGUAGE.into(),
            Self::Bash => tree_sitter_bash::LANGUAGE.into(),
        }
    }

    /// Languages whose files declare symbols (a JSON file only has a syntax to check).
    pub fn has_symbols(self) -> bool {
        !matches!(self, Self::Json | Self::Bash)
    }
}

/// Parses `text`. `None` only if the grammar cannot be loaded or the parser gives up.
pub fn parse(lang: Lang, text: &str) -> Option<Tree> {
    let mut parser = Parser::new();

    parser.set_language(&lang.grammar()).ok()?;
    parser.parse(text, None)
}

/// A syntax error: where, and a few characters of what is there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub line: usize,
    pub column: usize,
    pub near: String,
    pub missing: bool,
}

/// Up to `limit` syntax errors of a parsed file, in source order. An empty list means the file parses cleanly.
pub fn syntax_errors(tree: &Tree, text: &str, limit: usize) -> Vec<SyntaxError> {
    let mut found = Vec::new();

    if !tree.root_node().has_error() {
        return found;
    }

    collect_errors(tree.root_node(), text, limit, &mut found);
    found
}

fn collect_errors(node: Node, text: &str, limit: usize, found: &mut Vec<SyntaxError>) {
    if found.len() >= limit {
        return;
    }

    if node.is_error() || node.is_missing() {
        let start = node.start_byte();
        let end = node.end_byte().min(start + 40);
        let near = text.get(start..end).unwrap_or_default().lines().next().unwrap_or_default().trim().to_string();

        found.push(SyntaxError { line: node.start_position().row + 1, column: node.start_position().column + 1, near: if node.is_missing() { format!("missing `{}`", node.kind()) } else { near }, missing: node.is_missing() });

        if node.is_error() {
            return;
        }
    }

    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.has_error() || child.is_missing() {
            collect_errors(child, text, limit, found);
        }
    }
}

/// The syntax errors of a text in a language; `None` when the language is unknown to the graph (nothing to say).
pub fn check(path: &str, text: &str) -> Option<Vec<SyntaxError>> {
    let lang = Lang::of(path)?;
    let tree = parse(lang, text)?;

    Some(syntax_errors(&tree, text, 5))
}

/// One declaration.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Def {
    /// `fn`, `struct`, `enum`, `trait`, `impl`, `mod`, `type`, `const`, `macro`, `class`, `interface`, `method`, `def`, ...
    pub kind: String,
    pub name: String,
    /// `Type::method` (Rust, PHP), `Class.method` (Python, JS, TS), `Type.Method` (Go) - the name with its enclosing scopes.
    pub qualified: String,
    pub start_line: usize,
    pub end_line: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    /// SHA-256 of the declaration's exact bytes.
    pub hash: String,
    /// SHA-256 of the same with every run of whitespace collapsed - what survives a reformat.
    pub norm_hash: String,
}

/// A place a name is used.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Reference {
    pub name: String,
    pub line: usize,
    /// The name is being called (or constructed, or invoked as a macro) here.
    pub call: bool,
}

pub fn sha(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// The text with every run of whitespace made a single space.
pub fn normalised(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn text_of<'a>(node: Node, source: &'a str) -> &'a str {
    source.get(node.start_byte()..node.end_byte()).unwrap_or_default()
}

fn name_of(node: Node, source: &str) -> Option<String> {
    node.child_by_field_name("name").map(|name| text_of(name, source).to_string()).filter(|name| !name.is_empty())
}

/// What kind of declaration a node is, for this language - `None` when it declares nothing.
fn kind_of(lang: Lang, node: Node) -> Option<&'static str> {
    let kind = node.kind();

    match lang {
        Lang::Rust => match kind {
            "function_item" | "function_signature_item" => Some("fn"),
            "struct_item" => Some("struct"),
            "enum_item" => Some("enum"),
            "trait_item" => Some("trait"),
            "type_item" => Some("type"),
            "mod_item" => Some("mod"),
            "const_item" | "static_item" => Some("const"),
            "macro_definition" => Some("macro"),
            _ => None,
        },
        Lang::Python => match kind {
            "function_definition" => Some("def"),
            "class_definition" => Some("class"),
            _ => None,
        },
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => match kind {
            "function_declaration" | "generator_function_declaration" => Some("fn"),
            "class_declaration" | "abstract_class_declaration" => Some("class"),
            "method_definition" | "abstract_method_signature" | "method_signature" => Some("method"),
            "interface_declaration" => Some("interface"),
            "type_alias_declaration" => Some("type"),
            "enum_declaration" => Some("enum"),
            _ => None,
        },
        Lang::Go => match kind {
            "function_declaration" => Some("fn"),
            "method_declaration" => Some("method"),
            "type_spec" => Some("type"),
            _ => None,
        },
        Lang::Php => match kind {
            "function_definition" => Some("fn"),
            "method_declaration" => Some("method"),
            "class_declaration" => Some("class"),
            "interface_declaration" => Some("interface"),
            "trait_declaration" => Some("trait"),
            _ => None,
        },
        Lang::Json | Lang::Bash => None,
    }
}

/// Whether a declaration opens a scope that names its members (`impl Foo { fn bar }` is `Foo::bar`).
fn scope_name(lang: Lang, node: Node, source: &str) -> Option<String> {
    match (lang, node.kind()) {
        (Lang::Rust, "impl_item") => node.child_by_field_name("type").map(|target| {
            /* `impl<T> Foo<T>` is `Foo` */
            let full = text_of(target, source);

            full.split(['<', ' ']).next().unwrap_or(full).to_string()
        }),
        (Lang::Rust, "trait_item") | (Lang::Rust, "mod_item") => name_of(node, source),
        (Lang::Python, "class_definition") => name_of(node, source),
        (Lang::JavaScript | Lang::TypeScript | Lang::Tsx, "class_declaration" | "abstract_class_declaration" | "interface_declaration") => name_of(node, source),
        (Lang::Php, "class_declaration" | "interface_declaration" | "trait_declaration") => name_of(node, source),
        _ => None,
    }
}

/// Go's methods name their receiver type: `func (s *Server) Start()` is `Server.Start`.
fn go_receiver(node: Node, source: &str) -> Option<String> {
    let receiver = node.child_by_field_name("receiver")?;
    let text = text_of(receiver, source);
    let cleaned: String = text.trim_matches(|c| c == '(' || c == ')').split_whitespace().last().unwrap_or_default().trim_start_matches('*').to_string();

    Some(cleaned.split('[').next().unwrap_or_default().to_string()).filter(|name| !name.is_empty())
}

fn separator(lang: Lang) -> &'static str {
    match lang {
        Lang::Rust | Lang::Php => "::",
        _ => ".",
    }
}

/// Every declaration of a file, in source order.
pub fn definitions(lang: Lang, tree: &Tree, source: &str) -> Vec<Def> {
    let mut found = Vec::new();

    walk(lang, tree.root_node(), source, &mut Vec::new(), &mut found);
    found
}

fn walk(lang: Lang, node: Node, source: &str, scopes: &mut Vec<(String, &'static str)>, found: &mut Vec<Def>) {
    let mut pushed = false;

    /* JS/TS: `const handler = (a) => …` and `const f = function () {}` declare functions. */
    let arrow = matches!(lang, Lang::JavaScript | Lang::TypeScript | Lang::Tsx)
        && node.kind() == "variable_declarator"
        && node.child_by_field_name("value").is_some_and(|value| matches!(value.kind(), "arrow_function" | "function_expression" | "function"));

    if let Some(kind) = if arrow { Some("fn") } else { kind_of(lang, node) } {
        let name = name_of(node, source);

        if let Some(name) = name {
            let mut qualified_scopes: Vec<String> = scopes.iter().map(|(name, _)| name.clone()).collect();

            if lang == Lang::Go && node.kind() == "method_declaration" {
                if let Some(receiver) = go_receiver(node, source) {
                    qualified_scopes = vec![receiver];
                }
            }

            let qualified = if qualified_scopes.is_empty() { name.clone() } else { format!("{}{}{}", qualified_scopes.join(separator(lang)), separator(lang), name) };
            /* A Python function nested in a class is a method; in a function it is a plain def. */
            /* only a class (Python) or an impl/trait (Rust) makes a function a method - a module does not */
            let in_type = scopes.last().is_some_and(|(_, scope_kind)| *scope_kind != "mod");
            let kind = if in_type && (kind == "def" || (kind == "fn" && lang == Lang::Rust)) { "method" } else { kind };
            let text = text_of(node, source);

            found.push(Def {
                kind: kind.to_string(),
                name,
                qualified,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_byte: node.start_byte(),
                end_byte: node.end_byte(),
                hash: sha(text),
                norm_hash: sha(&normalised(text)),
            });
        }
    }

    if let Some(scope) = scope_name(lang, node, source) {
        /* the impl block itself is a declaration worth listing for Rust, as the old extractor did */
        if lang == Lang::Rust && node.kind() == "impl_item" {
            let text = text_of(node, source);

            found.push(Def {
                kind: "impl".to_string(),
                name: scope.clone(),
                qualified: format!("impl {scope}"),
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_byte: node.start_byte(),
                end_byte: node.end_byte(),
                hash: sha(text),
                norm_hash: sha(&normalised(text)),
            });
        }

        scopes.push((scope, if node.kind() == "mod_item" { "mod" } else { "type" }));
        pushed = true;
    }

    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        walk(lang, child, source, scopes, found);
    }

    if pushed {
        scopes.pop();
    }
}

/// Every use of a name: identifiers, with whether each is being called.
pub fn references(tree: &Tree, source: &str) -> Vec<Reference> {
    let mut found = Vec::new();

    visit(tree.root_node(), source, &mut found);
    found
}

fn visit(node: Node, source: &str, found: &mut Vec<Reference>) {
    let kind = node.kind();

    if matches!(kind, "identifier" | "type_identifier" | "property_identifier" | "field_identifier" | "shorthand_property_identifier" | "name" | "scoped_identifier") && node.child_count() == 0 {
        let name = text_of(node, source);

        if !name.is_empty() && name.len() <= 80 {
            let call = (0..3).scan(node, |current, _| {
                *current = current.parent()?;

                Some(current.kind().to_string())
            }).any(|parent| parent.contains("call") || parent.contains("invocation") || parent == "new_expression" || parent == "object_creation_expression");

            found.push(Reference { name: name.to_string(), line: node.start_position().row + 1, call });
        }
    }

    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        visit(child, source, found);
    }
}

/// What a file imports, as the module text (`./store`, `crate::trust::policy`, `os.path`).
pub fn imports(lang: Lang, tree: &Tree, source: &str) -> Vec<String> {
    let mut found = Vec::new();

    collect_imports(lang, tree.root_node(), source, &mut found);
    found.sort();
    found.dedup();
    found
}

fn collect_imports(lang: Lang, node: Node, source: &str, found: &mut Vec<String>) {
    let kind = node.kind();

    let module = match (lang, kind) {
        (Lang::Rust, "use_declaration") => node.child_by_field_name("argument").map(|arg| text_of(arg, source).to_string()),
        (Lang::Rust, "mod_item") if node.child_by_field_name("body").is_none() => name_of(node, source),
        (Lang::Python, "import_statement") => Some(text_of(node, source).trim_start_matches("import").trim().to_string()),
        (Lang::Python, "import_from_statement") => node.child_by_field_name("module_name").map(|module| text_of(module, source).to_string()),
        (Lang::JavaScript | Lang::TypeScript | Lang::Tsx, "import_statement" | "export_statement") => node.child_by_field_name("source").map(|module| text_of(module, source).trim_matches(|c| c == '"' || c == '\'' || c == '`').to_string()),
        (Lang::JavaScript | Lang::TypeScript | Lang::Tsx, "call_expression") if text_of(node, source).starts_with("require(") => node.child_by_field_name("arguments").map(|args| text_of(args, source).trim_matches(|c| c == '(' || c == ')' || c == '"' || c == '\'').to_string()),
        (Lang::Go, "import_spec") => node.child_by_field_name("path").map(|path| text_of(path, source).trim_matches('"').to_string()),
        _ => None,
    };

    if let Some(module) = module.filter(|module| !module.is_empty()) {
        found.push(module);
    }

    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        collect_imports(lang, child, source, found);
    }
}

#[cfg(test)]
mod tests;
