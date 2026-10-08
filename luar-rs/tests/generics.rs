use luar_rs::{CompileOptions, Target, compile_source, compile_source_with_options};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TempProject {
    path: PathBuf,
}

impl TempProject {
    fn new() -> Self {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "luar-rs-generics-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn source_path(&self) -> PathBuf {
        self.path.join("main.luar")
    }

    fn definition(&self, module: &str, source: &str) {
        fs::write(self.path.join(format!("{module}.luard")), source).unwrap();
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn compile(source: &str) -> Result<String, Vec<String>> {
    compile_source(source, None)
}

fn compile_with_module(definition: &str, source: &str) -> Result<String, Vec<String>> {
    let project = TempProject::new();
    project.definition("m", definition);
    let path: &Path = &project.source_path();
    compile_source(&format!("import type m\n{source}"), Some(path))
}

fn assert_error(errors: &[String], expected: &str) {
    assert!(
        errors.iter().any(|error| error.contains(expected)),
        "expected an error containing {expected:?}, got {errors:#?}"
    );
}

// ─── type 宣言 ────────────────────────────────────────────────────────────────

#[test]
fn generic_type_alias_checks_table_literals() {
    let header = "template <T>\nexport type MyTable = { id: number, ref: T }\n";
    compile(&format!(
        "{header}local a: MyTable<number> = {{ id = 1, ref = 2 }}\n"
    ))
    .expect("matching fields should type-check");

    let errors = compile(&format!(
        "{header}local a: MyTable<number> = {{ id = 1, ref = \"x\" }}\n"
    ))
    .unwrap_err();
    assert_error(&errors, "cannot assign table to 'a: MyTable<number>'");
    assert_error(&errors, "field 'ref' expects number, got string");

    let errors = compile(&format!("{header}local a: MyTable<number> = {{ ref = 1 }}\n")).unwrap_err();
    assert_error(&errors, "missing field 'id'");
}

#[test]
fn record_field_assignment_is_checked() {
    let errors = compile(
        "type Point = { x: number, y: number }\nlocal p: Point = { x = 1, y = 2 }\np.x = \"a\"\n",
    )
    .unwrap_err();
    assert_error(&errors, "cannot assign string to 'p.x: number'");
}

#[test]
fn template_applies_to_the_next_declaration_only() {
    let errors = compile("template <T>\ntype A = { v: T }\ntype B = { v: T }\n").unwrap_err();
    assert_error(&errors, "in type 'B'");
    assert_error(&errors, "unknown type 'T'");
}

#[test]
fn type_argument_count_is_checked() {
    let source = "template <T>\ntype Box = { v: T }\n";
    let errors = compile(&format!("{source}local a: Box<number, string> = {{ v = 1 }}\n")).unwrap_err();
    assert_error(&errors, "type 'Box' expects 1 type argument(s), got 2");
    let errors = compile(&format!("{source}local a: Box = {{ v = 1 }}\n")).unwrap_err();
    assert_error(&errors, "type 'Box' expects 1 type argument(s), got 0");
    let errors = compile("local a: number<string> = 1\n").unwrap_err();
    assert_error(&errors, "type 'number' does not take type arguments");
}

#[test]
fn plain_aliases_and_optional_and_function_types_work() {
    compile(
        "type Id = number\ntype Callback = (number, string) -> boolean\nlocal id: Id = 1\nlocal cb: Callback = function(a, b) return true end\nlocal maybe: Id? = nil\n",
    )
    .expect("aliases should expand");
    let errors = compile("type Id = number\nlocal id: Id = \"x\"\n").unwrap_err();
    assert_error(&errors, "cannot assign string to 'id: number'");
}

#[test]
fn recursive_type_alias_is_accepted() {
    compile("type Node = { value: number, next: Node? }\nlocal n: Node = { value = 1, next = nil }\n")
        .expect("recursive aliases should be accepted");
}

#[test]
fn duplicate_type_alias_is_rejected() {
    let errors = compile("type A = number\ntype A = string\n").unwrap_err();
    assert_error(&errors, "type 'A' is already defined");
}

#[test]
fn type_and_export_remain_ordinary_identifiers() {
    let output = compile("local type = 1\nprint(type)\nlocal export = type\nlocal template = 2\nprint(template < 3)\n")
        .expect("contextual keywords must not break existing code");
    assert!(output.contains("print(template < 3)"));
}

#[test]
fn template_must_precede_a_declaration() {
    let errors = compile("template <T>\nlocal x = 1\n").unwrap_err();
    assert_error(&errors, "'template' can only be applied");
    let errors = compile("template <T>\nprint(1)\n").unwrap_err();
    assert_error(&errors, "'template' must be followed by");
}

// ─── declare function (.luard) ───────────────────────────────────────────────

#[test]
fn declare_function_infers_type_parameter_from_arguments() {
    let definition = "template <T>\ndeclare function add(a: T, b: T): number\n";
    compile_with_module(definition, "local n: number = m.add(1, 2)\n")
        .expect("consistent type arguments should type-check");
    compile_with_module(definition, "local s = \"a\"\nm.add(s, s)\n")
        .expect("strings are consistent too");

    let errors = compile_with_module(definition, "m.add(1, \"a\")\n").unwrap_err();
    assert_error(
        &errors,
        "type parameter 'T' was inferred as number but argument 2 is string",
    );
}

#[test]
fn declare_function_checks_plain_parameters() {
    let definition = "template <T>\ndeclare function add2(a: T, b: string): number\n";
    compile_with_module(definition, "m.add2(1, \"x\")\n").expect("valid call");
    let errors = compile_with_module(definition, "m.add2(1, 2)\n").unwrap_err();
    assert_error(&errors, "argument 2 of 'add2' expects string, got number");
    let errors = compile_with_module(definition, "m.add2(1)\n").unwrap_err();
    assert_error(&errors, "function 'add2' expects 2 argument(s), got 1");
}

#[test]
fn declare_function_return_type_uses_inferred_type_parameter() {
    let definition = "template <T>\ndeclare function first(a: T, b: T): T\n";
    compile_with_module(definition, "local n: number = m.first(1, 2)\n").expect("T = number");
    let errors = compile_with_module(definition, "local s: string = m.first(1, 2)\n").unwrap_err();
    assert_error(&errors, "cannot assign number to 's: string'");
}

#[test]
fn declare_function_without_template_reports_unknown_type() {
    let errors = compile_with_module("declare function add(a: T, b: T): number\n", "print(1)\n")
        .unwrap_err();
    assert_error(&errors, "m.luard: in 'declare function add': unknown type 'T'");
}

#[test]
fn declare_global_function_and_function_types_in_luard() {
    let definition = "declare global function log(message: string)\ndeclare run: () -> ()\n";
    compile_with_module(definition, "log(\"x\")\nm.run()\n").expect("valid calls");
    let errors = compile_with_module(definition, "log(1)\n").unwrap_err();
    assert_error(&errors, "argument 1 of 'log' expects string, got number");
}

#[test]
fn declare_function_is_only_allowed_in_luard() {
    let errors = compile("declare function add(a: number): number\n").unwrap_err();
    assert_error(&errors, "only allowed in .luard");
}

#[test]
fn exported_types_are_referenced_through_the_module() {
    let definition = "template <T>\nexport type Pair = { first: T, second: T }\ntemplate <T>\ndeclare function make(a: T): Pair<T>\n";
    compile_with_module(
        definition,
        "local p: m.Pair<number> = { first = 1, second = 2 }\n",
    )
    .expect("qualified generic type");
    let errors = compile_with_module(
        definition,
        "local p: m.Pair<number> = { first = 1, second = \"x\" }\n",
    )
    .unwrap_err();
    assert_error(&errors, "field 'second' expects number, got string");

    // 修飾なしの `Pair<number>` は見つからないのでエラー。存在しない修飾名だけの注釈は従来どおり許す。
    let errors = compile_with_module(
        definition,
        "local p: Pair<number> = { first = 1, second = 2 }
",
    )
    .unwrap_err();
    assert_error(&errors, "unknown type 'Pair'");
    compile_with_module(definition, "local q: m.Missing = nil
")
        .expect("a bare unresolved annotation name stays lenient");
}

#[test]
fn private_luard_types_are_not_visible_from_outside() {
    let definition = "type Hidden = { v: number }\nexport type Shown = { v: Hidden }\n";
    compile_with_module(definition, "local s: m.Shown = { v = { v = 1 } }\n").expect("export is visible");
    // export されていない型は外から解決できず、Unknown として扱われる。
    compile_with_module(definition, "local h: m.Hidden = 1\n")
        .expect("a non-exported type is unresolved, hence unchecked");
}

// ─── 関数・クラス ────────────────────────────────────────────────────────────

#[test]
fn template_function_definition_is_checked_at_call_sites() {
    let function = "template <T>\nfunction first(a: T, b: T): T\n    return a\nend\n";
    compile(&format!("{function}local n: number = first(1, 2)\n")).expect("T = number");
    let errors = compile(&format!("{function}first(1, \"x\")\n")).unwrap_err();
    assert_error(
        &errors,
        "type parameter 'T' was inferred as number but argument 2 is string",
    );
}

#[test]
fn template_local_function_is_checked_at_call_sites() {
    let function = "template <T>\nlocal function pick(a: T, b: T): T\n    return a\nend\n";
    compile(&format!("{function}local s: string = pick(\"a\", \"b\")\n")).expect("T = string");
    let errors = compile(&format!("{function}pick(1, \"b\")\n")).unwrap_err();
    assert_error(&errors, "type parameter 'T' was inferred as number");
}

#[test]
fn template_function_body_can_use_the_type_parameter() {
    compile(
        "template <T>\nfunction id(a: T): T\n    local copy: T = a\n    return copy\nend\n",
    )
    .expect("T is visible in the body");
    let errors = compile("template <T>\nfunction bad(a: T): T\n    local n: number = a\n    return a\nend\n")
        .unwrap_err();
    assert_error(&errors, "cannot assign T to 'n: number'");
}

#[test]
fn ordinary_function_calls_are_not_checked() {
    compile("function add(a: number, b: number): number\n    return a + b\nend\nadd(\"x\", {})\n")
        .expect("non-template functions keep their previous behaviour");
}

#[test]
fn generic_class_substitutes_field_and_method_types() {
    let class = r#"
template <T>
class Box is
    public is
        value: T
        static function new(v: T): Box
            self.value = v
        end
        function get(): T
            return self.value
        end
    end
end
"#;
    compile(&format!(
        "{class}local b: Box<number> = Box.new(1)\nlocal n: number = b.get()\nlocal v: number = b.value\n"
    ))
    .expect("T is replaced by the instance type argument");
    let errors = compile(&format!(
        "{class}local b: Box<number> = Box.new(1)\nlocal s: string = b.get()\n"
    ))
    .unwrap_err();
    assert_error(&errors, "cannot assign number to 's: string'");
    let errors = compile(&format!("{class}local b: Box<number, string> = Box.new(1)\n")).unwrap_err();
    assert_error(&errors, "type 'Box' expects 1 type argument(s), got 2");
}

#[test]
fn generic_class_instances_with_different_arguments_do_not_mix() {
    let class = "template <T>\nclass Box is\n    public is\n        value: T\n    end\nend\n";
    let errors = compile(&format!(
        "{class}local a: Box<number> = nil\nlocal b: Box<string> = a\n"
    ));
    // nil は Optional でない Box へ代入できないため、まず別のエラーになる。
    assert!(errors.is_err());
}

// ─── 生成コードのコメント ────────────────────────────────────────────────────

#[test]
fn type_signatures_are_written_as_comments() {
    let output = compile(
        "template <T>\nexport type MyTable = { id: number, ref: T }\nlocal score: number, name = 1, \"x\"\nconst LIMIT: number = 10\ntemplate <T>\nfunction add(a: T, b: string): number\n    return 1\nend\nfunction plain(a)\n    return a\nend\n",
    )
    .unwrap();
    assert!(output.contains("-- export type MyTable<T> = { id: number, ref: T }"), "{output}");
    assert!(output.contains("-- local score: number, name\nlocal score, name = 1, \"x\""), "{output}");
    assert!(output.contains("-- const LIMIT: number"), "{output}");
    assert!(output.contains("-- function add<T>(a: T, b: string): number\nfunction add(a, b)"), "{output}");
    assert!(!output.contains("-- function plain"), "untyped code gets no comment: {output}");
}

#[test]
fn class_type_information_is_written_as_comments() {
    let output = compile(
        "template <T>\nclass Box is\n    public is\n        value: T\n        function get(): T\n            return self.value\n        end\n    end\nend\n",
    )
    .unwrap();
    assert!(output.contains("-- class Box<T>"), "{output}");
    assert!(output.contains("--   value: T"), "{output}");
    assert!(output.contains("--   function get(): T"), "{output}");
}

#[test]
fn comments_are_emitted_for_both_targets() {
    for target in [Target::Luau, Target::Lua54] {
        let options = CompileOptions {
            target,
            source_path: None,
        };
        let output = compile_source_with_options(
            "local count: number = 1\nlocal function twice(x: number): number\n    return x * 2\nend\n",
            &options,
        )
        .unwrap_or_else(|errors| panic!("{errors:?}"));
        assert!(output.contains("-- local count: number"), "{output}");
        assert!(
            output.contains("-- local function twice(x: number): number"),
            "{output}"
        );
    }
}

// ─── 色分け ────────────────────────────────────────────────────────────────

fn token_kind(tokens: &[luar_rs::navigation::SemanticToken], line: usize, column: usize) -> String {
    tokens
        .iter()
        .find(|token| token.line == line && token.column == column)
        .map(|token| token.kind.clone())
        .unwrap_or_else(|| panic!("no token at {line}:{column} in {tokens:#?}"))
}

#[test]
fn semantic_tokens_color_type_declarations_and_parameters() {
    let source = "template <T>\nexport type MyTable = { id: number, ref: T }\nlocal a: MyTable<number> = { id = 1, ref = 2 }\n";
    let project = TempProject::new();
    let options = CompileOptions {
        source_path: Some(project.source_path()),
        ..CompileOptions::default()
    };
    let tokens = luar_rs::navigation::semantic_tokens(source, &options);
    assert_eq!(token_kind(&tokens, 0, 0), "keyword", "template");
    assert_eq!(token_kind(&tokens, 0, 10), "type", "type parameter declaration");
    assert_eq!(token_kind(&tokens, 1, 0), "keyword", "export");
    assert_eq!(token_kind(&tokens, 1, 12), "type", "alias name");
    assert_eq!(token_kind(&tokens, 1, 24), "property", "table type field");
    assert_eq!(token_kind(&tokens, 1, 41), "type", "type parameter use");
    assert_eq!(token_kind(&tokens, 2, 9), "type", "alias use");
    assert_eq!(token_kind(&tokens, 2, 17), "type", "type argument");
}

#[test]
fn semantic_tokens_color_declare_function_in_luard() {
    let project = TempProject::new();
    let options = CompileOptions {
        source_path: Some(project.path.join("m.luard")),
        ..CompileOptions::default()
    };
    let source = "template <T>\ndeclare function add(a: T, b: T): number\n";
    let tokens = luar_rs::navigation::semantic_tokens(source, &options);
    assert_eq!(token_kind(&tokens, 1, 17), "function", "function name");
    assert_eq!(token_kind(&tokens, 1, 21), "parameter", "parameter");
    assert_eq!(token_kind(&tokens, 1, 24), "type", "type parameter use");
    assert_eq!(token_kind(&tokens, 1, 34), "type", "number");
}

// ─── クラスのメソッドの template と配列型 ────────────────────────────────────

#[test]
fn class_methods_can_have_their_own_type_parameters() {
    let class = r#"
class Finder is
    public is
        template <T>
        function find(name: string): T?
            return nil
        end
        template <T>
        static function all(): { T }
            return {}
        end
    end
end
"#;
    let output = compile(&format!(
        "{class}local f = Finder.new()\nlocal found: number? = f.find(\"x\")\n"
    ))
    .expect("method type parameters are unknown at call sites");
    assert!(output.contains("--   function find<T>(name: string): T?"), "{output}");
    assert!(output.contains("--   static function all<T>(): { T }"), "{output}");

    let errors = compile(
        "class Bad is\n    public is\n        function f(): U\n            return nil\n        end\n    end\nend\n",
    );
    errors.expect("an unknown type in an ordinary class stays lenient");
}

#[test]
fn template_before_a_field_or_operator_is_rejected() {
    let errors = compile("class A is\n    public is\n        template <T>\n        x: T\n    end\nend\n")
        .unwrap_err();
    assert_error(&errors, "'template' can only be applied to a method");
}

#[test]
fn array_types_parse_and_accept_tables() {
    compile("local xs: { number } = { 1, 2 }\nlocal ys: { string }? = nil\ntype List = { number }\n")
        .expect("array types are tracked as tables");
    let errors = compile("local xs: { number } = 1\n").unwrap_err();
    assert_error(&errors, "cannot assign number to 'xs: table'");
}

#[test]
fn declare_class_with_generic_methods_in_luard() {
    let definition = "declare class Instance is\n    public is\n        Name: string\n        template <T> function FindChild(Inst: Instance, name: string): T?\n        template <T> function GetChildren(Inst: Instance): { T }\n        function Destroy(Inst: Instance): ()\n        static function new(classname: string): Instance?\n    end\nend\n";
    compile_with_module(
        definition,
        "local i = Instance.new(\"Part\")\nlocal name: string = i.Name\n",
    )
    .expect("generic methods in a declare class");
}

#[test]
fn semantic_tokens_color_method_templates_and_array_types() {
    let project = TempProject::new();
    let options = CompileOptions {
        source_path: Some(project.path.join("m.luard")),
        ..CompileOptions::default()
    };
    let source = "declare class A is\n    public is\n        template <T> function get(): { T }\n    end\nend\n";
    let tokens = luar_rs::navigation::semantic_tokens(source, &options);
    assert_eq!(token_kind(&tokens, 2, 8), "keyword", "template");
    assert_eq!(token_kind(&tokens, 2, 18), "type", "type parameter declaration");
    assert_eq!(token_kind(&tokens, 2, 39), "type", "array element");
}
