use luar_rs::navigation::{definition, semantic_tokens};
use luar_rs::{CompileOptions, Severity, analyze_source_with_options, compile_source};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TempProject {
    root: PathBuf,
}

impl TempProject {
    fn new() -> Self {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("luar-rs-imports-{}-{id}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn write(&self, relative: &str, source: &str) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, source).unwrap();
        path
    }

    fn options(&self, relative: &str) -> CompileOptions {
        CompileOptions {
            source_path: Some(self.root.join(relative)),
            ..CompileOptions::default()
        }
    }

    fn compile(&self, relative: &str, source: &str) -> Result<String, Vec<String>> {
        compile_source(source, Some(&self.root.join(relative)))
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

const DEFS: &str = "declare answer: number\ndeclare class Dog is\n    public is\n        name: string\n    end\nend\n";

fn assert_error(errors: &[String], expected: &str) {
    assert!(
        errors.iter().any(|error| error.contains(expected)),
        "expected an error containing {expected:?}, got {errors:#?}"
    );
}

// ─── import type ... from ───────────────────────────────────────────────────

#[test]
fn import_from_a_parent_directory() {
    let project = TempProject::new();
    project.write("defs/m.luard", DEFS);
    let output = project
        .compile(
            "src/main.luar",
            "import type m from \"../defs/m.luard\"\nlocal n: number = m.answer\nprint(n)\n",
        )
        .expect("a definition in a sibling directory");
    assert!(!output.contains("import"), "{output}");
    let errors = project
        .compile(
            "src/main.luar",
            "import type m from \"../defs/m.luard\"\nlocal n: string = m.answer\n",
        )
        .unwrap_err();
    assert_error(&errors, "cannot assign number to 'n: string'");
}

#[test]
fn import_from_subdirectories_and_several_levels_up() {
    let project = TempProject::new();
    project.write("types/deep/m.luard", DEFS);
    project.compile("main.luar", "import type m from \"./types/deep/m.luard\"\nprint(m.answer)\n")
        .expect("a subdirectory");
    project.write("a/b/c/x.luar", "");
    project
        .compile(
            "a/b/c/main.luar",
            "import type m from \"../../../types/deep/m.luard\"\nprint(m.answer)\n",
        )
        .expect("three levels up");
}

#[test]
fn the_binding_name_is_independent_of_the_file_name() {
    let project = TempProject::new();
    project.write("defs/love-types.luard", "declare global love: table\ndeclare version: string\n");
    project
        .compile(
            "main.luar",
            "import type engine from \"./defs/love-types.luard\"\nlocal v: string = engine.version\n",
        )
        .expect("the name `engine` qualifies the members");
}

#[test]
fn the_plain_form_still_reads_the_same_directory() {
    let project = TempProject::new();
    project.write("m.luard", DEFS);
    project
        .compile("main.luar", "import type m\nlocal n: number = m.answer\n")
        .expect("unchanged");
}

#[test]
fn classes_from_an_imported_path_are_available() {
    let project = TempProject::new();
    project.write("defs/m.luard", DEFS);
    project
        .compile(
            "src/main.luar",
            "import type m from \"../defs/m.luard\"\nlocal d = Dog.new()\nlocal s: string = d.name\n",
        )
        .expect("declared classes are global");
}

#[test]
fn invalid_paths_are_reported_on_the_import_line() {
    let project = TempProject::new();
    project.write("defs/m.luard", DEFS);
    let absolute = if cfg!(windows) { "C:/defs/m.luard" } else { "/defs/m.luard" };
    let errors = project
        .compile("main.luar", &format!("-- c\nimport type m from \"{absolute}\"\n"))
        .unwrap_err();
    assert_error(&errors, "import type paths must be relative");

    let errors = project
        .compile("main.luar", "import type m from \"./defs/m.lua\"\n")
        .unwrap_err();
    assert_error(&errors, "import type only accepts .luard files");

    let errors = project
        .compile("main.luar", "import type m from \"./defs/missing.luard\"\n")
        .unwrap_err();
    assert_error(&errors, "cannot read module definition");

    let diagnostics = analyze_source_with_options(
        "-- c\nimport type m from \"./defs/m.lua\"\nprint(1)\n",
        &project.options("main.luar"),
    )
    .unwrap_err();
    assert_eq!(diagnostics[0].line, 2);
    assert_eq!(diagnostics[0].severity, Severity::Error);
}

#[test]
fn a_missing_from_path_is_a_syntax_error() {
    let project = TempProject::new();
    let errors = project.compile("main.luar", "import type m from m\n").unwrap_err();
    assert_error(&errors, "expected a path string after 'from'");
}

#[test]
fn a_same_named_include_needs_no_definition_file() {
    let project = TempProject::new();
    project.write("lib/m.luar", "local m = {}\nm.answer = 1\nreturn m\n");
    project
        .compile(
            "main.luar",
            "import type m from \"./lib/m.luard\"\nlocal m = !include(\"./lib/m.luar\")\nprint(m.answer)\n",
        )
        .expect("the included source supplies the declarations");
}

// ─── !include ───────────────────────────────────────────────────────────────

#[test]
fn include_reaches_parent_and_nested_directories() {
    let project = TempProject::new();
    project.write("shared/util.luar", "local util = {}\nutil.one = 1\nreturn util\n");
    let output = project
        .compile(
            "app/main.luar",
            "local util = !include(\"../shared/util.luar\")\nprint(util.one)\n",
        )
        .expect("a parent directory");
    assert!(output.contains("util.one = 1"), "{output}");
}

#[test]
fn nested_includes_are_relative_to_the_including_file() {
    let project = TempProject::new();
    project.write("lib/inner/leaf.luar", "local leaf = {}\nleaf.v = 7\nreturn leaf\n");
    project.write(
        "lib/mid.luar",
        "local leaf = !include(\"./inner/leaf.luar\")\nlocal mid = {}\nmid.leaf = leaf\nreturn mid\n",
    );
    project
        .compile("main.luar", "local mid = !include(\"./lib/mid.luar\")\nprint(mid.leaf.v)\n")
        .expect("the inner path is relative to lib/mid.luar");
}

#[test]
fn absolute_include_paths_are_still_rejected() {
    let project = TempProject::new();
    let absolute = if cfg!(windows) { "C:/x.luar" } else { "/x.luar" };
    let errors = project
        .compile("main.luar", &format!("local x = !include(\"{absolute}\")\n"))
        .unwrap_err();
    assert_error(&errors, "must be relative to the including source file");
}

// ─── エディタ向け ───────────────────────────────────────────────────────────

#[test]
fn tokens_color_the_binding_and_from() {
    let project = TempProject::new();
    project.write("defs/m.luard", DEFS);
    let source = "import type m from \"../defs/m.luard\"\nprint(m.answer)\n";
    let tokens = semantic_tokens(source, &project.options("src/main.luar"));
    let kind = |line: usize, column: usize| {
        tokens
            .iter()
            .find(|token| token.line == line && token.column == column)
            .map(|token| token.kind.clone())
    };
    assert_eq!(kind(0, 12).as_deref(), Some("namespace"), "binding name");
    assert_eq!(kind(0, 14).as_deref(), Some("keyword"), "from");
    assert_eq!(kind(1, 6).as_deref(), Some("namespace"), "the use of m");
}

#[test]
fn definition_opens_the_imported_file_from_the_path_and_the_name() {
    let project = TempProject::new();
    let target = project.write("defs/m.luard", DEFS);
    let source = "import type m from \"../defs/m.luard\"\nprint(m.answer)\n";
    let options = project.options("src/main.luar");

    let on_path = definition(source, source.find("../defs").unwrap() + 2, &options);
    assert_eq!(on_path.len(), 1, "{on_path:?}");
    assert!(same_file(&on_path[0].file, &target), "{on_path:?}");

    let on_name = definition(source, source.find("m from").unwrap(), &options);
    assert_eq!(on_name.len(), 1, "{on_name:?}");
    assert!(same_file(&on_name[0].file, &target), "{on_name:?}");

    let on_member = definition(source, source.find("answer").unwrap() + 1, &options);
    assert_eq!(on_member.len(), 1, "{on_member:?}");
    assert_eq!(on_member[0].line, 0);
}

fn same_file(reported: &str, expected: &Path) -> bool {
    let reported = fs::canonicalize(reported);
    let expected = fs::canonicalize(expected);
    reported.is_ok() && reported.ok() == expected.ok()
}

#[test]
fn the_imports_command_reports_resolved_paths() {
    let project = TempProject::new();
    project.write("defs/m.luard", DEFS);
    let source = "import type m from \"../defs/m.luard\"\nimport type local_defs\nimport type bad from \"x.lua\"\n";
    let source_path = project.root.join("src/main.luar");
    let mut child = Command::new(env!("CARGO_BIN_EXE_luar"))
        .args(["imports", "--stdin", "--source-path"])
        .arg(&source_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(source.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let imports = report["imports"].as_array().unwrap();
    assert_eq!(imports.len(), 3);

    assert_eq!(imports[0]["name"], "m");
    assert_eq!(imports[0]["line"], 0);
    assert_eq!(imports[0]["column"], 12);
    let resolved = PathBuf::from(imports[0]["path"].as_str().unwrap());
    assert!(
        resolved.ends_with(Path::new("defs").join("m.luard")) && !resolved.to_string_lossy().contains(".."),
        "{resolved:?}"
    );

    let plain = PathBuf::from(imports[1]["path"].as_str().unwrap());
    assert!(plain.ends_with(Path::new("src").join("local_defs.luard")), "{plain:?}");

    assert!(imports[2]["path"].is_null());
    assert!(imports[2]["error"].as_str().unwrap().contains(".luard"));
}

// ─── モジュールとして返すもの ───────────────────────────────────────────────

const TABLE2: &str = "class table2 is\n    public is\n        static function add(items: {number}, amount: number)\n            for index, value in ipairs(items) do\n                items[index] = value + amount\n            end\n        end\n    end\nend\n";

#[test]
fn a_static_class_can_be_returned_as_the_module() {
    let project = TempProject::new();
    project.write("table2.luar", &format!("{TABLE2}\nreturn table2\n"));
    let output = project
        .compile(
            "main.luar",
            "local table2 = !include(\"./table2.luar\")\nlocal items = {1, 2}\ntable2.add(items, 1)\n",
        )
        .expect("a class is a valid module");
    assert!(output.contains("table2.add(items, 1)"), "{output}");
    assert!(output.contains("function table2.add(items: { number }, amount: number)"), "{output}");
}

#[test]
fn a_missing_return_suggests_the_class_name() {
    let project = TempProject::new();
    project.write("table2.luar", TABLE2);
    let errors = project
        .compile("main.luar", "local table2 = !include(\"./table2.luar\")\n")
        .unwrap_err();
    assert_error(&errors, "included source must end with standalone `return <identifier>`");
    assert_error(&errors, "add `return table2` at the end of the file");
}

#[test]
fn a_missing_return_suggests_the_last_top_level_name() {
    let project = TempProject::new();
    project.write(
        "lib.luar",
        "local helper = 1\nlocal function inner()\n    local nested = 2\nend\nlocal lib = {}\nlib.v = helper\n",
    );
    let errors = project
        .compile("main.luar", "local lib = !include(\"./lib.luar\")\n")
        .unwrap_err();
    assert_error(&errors, "add `return lib` at the end of the file");
}

#[test]
fn a_missing_return_without_any_declaration_keeps_the_plain_message() {
    let project = TempProject::new();
    project.write("empty.luar", "print(1)\n");
    let errors = project
        .compile("main.luar", "local empty = !include(\"./empty.luar\")\n")
        .unwrap_err();
    assert_error(&errors, "included source must end with standalone `return <identifier>`");
    assert!(
        !errors.iter().any(|error| error.contains("add `return")),
        "{errors:#?}"
    );
}
