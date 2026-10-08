use luar_rs::navigation::{SemanticToken, semantic_tokens};
use luar_rs::{CompileOptions, Target};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TempProject {
    path: PathBuf,
}

impl TempProject {
    fn new() -> Self {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("luar-hl-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn write(&self, name: &str, source: &str) {
        fs::write(self.path.join(name), source).unwrap();
    }

    fn tokens(&self, name: &str, source: &str) -> Vec<SemanticToken> {
        let options = CompileOptions {
            target: Target::Luau,
            source_path: Some(self.path.join(name)),
        };
        semantic_tokens(source, &options)
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn at(tokens: &[SemanticToken], line: usize, column: usize) -> (String, Vec<String>) {
    let token = tokens
        .iter()
        .find(|token| token.line == line && token.column == column)
        .unwrap_or_else(|| panic!("no token at {line}:{column} in {tokens:#?}"));
    (token.kind.clone(), token.modifiers.clone())
}

fn kind(tokens: &[SemanticToken], line: usize, column: usize) -> String {
    at(tokens, line, column).0
}

#[test]
fn names_inside_template_strings_are_resolved() {
    let project = TempProject::new();
    let source = "local name = \"x\"\nprint(`hi {name}!`)\n";
    let tokens = project.tokens("main.luar", source);
    // `name` は上の local へ解決される(変数)。
    assert_eq!(at(&tokens, 1, 11), ("variable".into(), vec![]));
}

#[test]
fn template_expressions_support_members_calls_and_standard_names() {
    let project = TempProject::new();
    let source = "local t = { n = 1 }\nprint(`{t.n} {math.floor(t.n)}`)\n";
    let tokens = project.tokens("main.luar", source);
    assert_eq!(kind(&tokens, 1, 8), "variable", "t");
    assert_eq!(kind(&tokens, 1, 10), "property", "n");
    assert_eq!(
        at(&tokens, 1, 14),
        ("namespace".into(), vec!["defaultLibrary".into()]),
        "math"
    );
    assert_eq!(
        at(&tokens, 1, 19),
        ("function".into(), vec!["defaultLibrary".into()]),
        "floor"
    );
}

#[test]
fn template_expression_columns_account_for_wide_characters_and_lines() {
    let project = TempProject::new();
    // 日本語(UTF-16で1文字ずつ)と、改行をまたぐテンプレート。
    let source = "local 名前 = 1\nlocal text = `こんにちは {名前}\n次 {text}`\n";
    let tokens = project.tokens("main.luar", source);
    assert_eq!(kind(&tokens, 1, 21), "variable", "名前 on the first line");
    assert_eq!(kind(&tokens, 2, 3), "variable", "text on the second line");
}

#[test]
fn escaped_braces_and_nested_braces_in_templates() {
    let project = TempProject::new();
    let source = "local a = 1\nprint(`\\{a} {a}`)\n";
    let tokens = project.tokens("main.luar", source);
    // `\{a}` はただの文字列。区切りとして働くのは後ろの `{a}` だけ。
    let names: Vec<_> = tokens.iter().filter(|token| token.line == 1 && token.length == 1).collect();
    assert_eq!(names.len(), 1, "{tokens:#?}");
    assert_eq!(names[0].column, 13);
}

#[test]
fn type_positions_use_the_type_kind_like_primitives() {
    let project = TempProject::new();
    project.write(
        "defs.luard",
        "type Arithmetic = Vector3 | number\ndeclare class Vector3 is\n    public is\n        zero: Vector3\n    end\nend\n",
    );
    let source = "import type defs\nlocal v: Vector3 = nil\nlocal w: Arithmetic = 1\nlocal p = Vector3.new()\n";
    let tokens = project.tokens("main.luar", source);
    assert_eq!(kind(&tokens, 1, 9), "type", "annotation: class name");
    assert_eq!(kind(&tokens, 2, 9), "type", "annotation: alias name");
    assert_eq!(kind(&tokens, 3, 10), "class", "an expression use of the class stays a class");
}

#[test]
fn declarations_in_a_definition_file_keep_their_kinds() {
    let project = TempProject::new();
    let source = "type Arithmetic = Vector3 | number\ndeclare class Vector3 is\n    public is\n        zero: Vector3\n    end\nend\n";
    let tokens = project.tokens("defs.luard", source);
    assert_eq!(kind(&tokens, 0, 0), "keyword", "type");
    assert_eq!(kind(&tokens, 0, 5), "type", "alias declaration");
    assert_eq!(kind(&tokens, 0, 18), "type", "forward reference to a class in a type position");
    assert_eq!(kind(&tokens, 1, 14), "class", "class declaration");
    assert_eq!(kind(&tokens, 3, 14), "type", "field type");
}

#[test]
fn imported_classes_and_modules_are_highlighted() {
    let project = TempProject::new();
    project.write(
        "RCBN.luard",
        "declare class Instance is\n    public is\n        static function new(classname: string): Instance?\n    end\nend\n",
    );
    let source = "import type RCBN\nlocal i = Instance.new(\"Part\")\n";
    let tokens = project.tokens("main.luar", source);
    assert_eq!(kind(&tokens, 0, 12), "namespace", "the imported module");
    assert_eq!(kind(&tokens, 1, 10), "class", "the declared class");
}
