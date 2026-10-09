use luar_rs::compile_source;
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
        let path = std::env::temp_dir().join(format!("luar-rs-unions-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
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
    fs::write(project.path.join("m.luard"), definition).unwrap();
    let path: &Path = &project.path.join("main.luar");
    compile_source(&format!("import type m\n{source}"), Some(path))
}

fn assert_error(errors: &[String], expected: &str) {
    assert!(
        errors.iter().any(|error| error.contains(expected)),
        "expected an error containing {expected:?}, got {errors:#?}"
    );
}

#[test]
fn a_union_accepts_each_of_its_members_only() {
    compile("local a: number | string = 1\nlocal b: number | string = \"x\"\n")
        .expect("both members are accepted");
    let errors = compile("local c: number | string = true\n").unwrap_err();
    assert_error(&errors, "cannot assign boolean to 'c: number | string'");
}

#[test]
fn a_union_value_needs_every_member_to_fit() {
    compile("local a: number | string = 1\nlocal b: number | string | boolean = a\n")
        .expect("a narrower union fits a wider one");
    let errors = compile("local a: number | string = 1\nlocal n: number = a\n").unwrap_err();
    assert_error(&errors, "cannot assign number | string to 'n: number'");
}

#[test]
fn nil_in_a_union_makes_it_optional() {
    compile("local a: number | nil = nil\nlocal b: (number | string)? = nil\nlocal c: number | string | nil = 1\n")
        .expect("`T | nil` is an optional type");
    let errors = compile("local a: (number | string)? = nil\nlocal n: number = a\n").unwrap_err();
    assert_error(&errors, "cannot assign (number | string)? to 'n: number'");
}

#[test]
fn union_aliases_and_function_types_work() {
    compile("type Id = number | string\nlocal a: Id = 1\nlocal b: Id = \"x\"\n").expect("alias of a union");
    let errors = compile("type Id = number | string\nlocal a: Id = {}\n").unwrap_err();
    assert_error(&errors, "cannot assign table to 'a: number | string'");
    compile("local f: (number) -> number | string = function(x) return x end\n")
        .expect("the return type takes the union");
}

#[test]
fn casts_involving_unions_need_one_related_member() {
    compile("local a: number | string = 1\nlocal n = a :: number\nlocal s = n :: number | string\n")
        .expect("narrowing and widening casts");
    let errors = compile("local a: number | string = 1\nlocal b = a :: boolean\n").unwrap_err();
    assert_error(&errors, "cannot cast number | string to boolean");
}

#[test]
fn operators_on_unions_are_not_rejected() {
    compile("local a: number | string = 1\nlocal b = a + 1\n").expect("members are not tracked");
}

#[test]
fn union_types_are_written_as_luau_annotations() {
    let output = compile("local a: number | string = 1\n").unwrap();
    assert!(output.contains("local a: number | string = 1"), "{output}");
}

#[test]
fn union_parameters_work_for_operator_methods_in_declarations() {
    let definition = r#"
type Arithmetic = Vector3 | number

declare class Vector3 is
    public is
        x: number
        static function new(x: number): Vector3
        function operator+(a: Vector3, b: Vector3): Vector3
        function operator*(a: Vector3, b: Vector3 | number): Vector3
        function operator/(a: Vector3, b: Arithmetic): Vector3
    end
end
"#;
    compile_with_module(
        definition,
        "local v = Vector3.new(1)\nlocal a = v * 2\nlocal b = v * v\nlocal c = v / 2\nlocal d = v + v\n",
    )
    .expect("both operands are accepted");
    let errors = compile_with_module(definition, "local v = Vector3.new(1)\nlocal a = v * \"x\"\n")
        .unwrap_err();
    assert_error(&errors, "operator '*' for 'Vector3' expects Vector3 | number, got string");
    let errors = compile_with_module(definition, "local v = Vector3.new(1)\nlocal a = v + 1\n")
        .unwrap_err();
    assert_error(&errors, "operator '+' for 'Vector3' expects Vector3, got number");
}

#[test]
fn template_applies_to_operator_methods() {
    let definition = r#"
declare class Vector3 is
    public is
        static function new(x: number): Vector3
        template <A>
        function operator*(a: Vector3, b: A): Vector3
        template <A>
        function operator/(a: Vector3, b: A): Vector3
    end
end
"#;
    compile_with_module(definition, "local v = Vector3.new(1)\nlocal a = v * 2\nlocal b = v / v\n")
        .expect("each operator has its own template");
}

#[test]
fn declare_global_class_gets_a_clear_message() {
    let errors = compile_with_module(
        "declare global class Instance is\n    public is\n        Name: string\n    end\nend\n",
        "print(1)\n",
    )
    .unwrap_err();
    assert_error(&errors, "a declared class is already global");
}

#[test]
fn declaration_errors_point_at_the_broken_member() {
    let errors = compile_with_module(
        "declare class A is\n    public is\n        x: number\n        template <T> function f(a number)\n    end\nend\n",
        "print(1)\n",
    )
    .unwrap_err();
    assert!(
        errors.iter().any(|error| error.contains("m.luard:4")),
        "the error should be at line 4: {errors:#?}"
    );
}

#[test]
fn a_type_declaration_inside_a_class_gets_a_clear_message() {
    let errors = compile_with_module(
        "declare class Vector3 is
    public is
        x: number
        type Arithmetic = Vector3 | number
        function operator*(a: Vector3, b: Arithmetic): Vector3
    end
end
",
        "print(1)
",
    )
    .unwrap_err();
    assert_error(&errors, "m.luard:4");
    assert_error(&errors, "a type declaration cannot be written inside a class");
}
