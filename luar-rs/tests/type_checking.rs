use luar_rs::{CompileOptions, check_source_with_options};

fn check(source: &str) -> Result<(), Vec<String>> {
    check_source_with_options(source, &CompileOptions::default())
        .map(|_| ())
        .map_err(|errors| errors.into_iter().map(|error| error.message).collect())
}

#[test]
fn rejects_arithmetic_with_inferred_incompatible_primitives() {
    let errors = check("local a: number = 1\nlocal b: string = 'A'\nlocal c = a + b\n")
        .expect_err("number + string must fail");
    assert!(
        errors
            .iter()
            .any(|error| error.contains("operator '+' expects number operands"))
    );
}

#[test]
fn rejects_incompatible_type_annotation() {
    let errors = check("local ignored = true\nlocal title: string = 42\n")
        .expect_err("annotated mismatch must fail");
    assert!(
        errors
            .iter()
            .any(|error| error.contains("cannot assign number to 'title: string'"))
    );
}

#[test]
fn reports_annotation_mismatch_on_its_declaration_line() {
    let errors = check_source_with_options(
        "local ignored = true\nlocal title: string = 42\n",
        &CompileOptions::default(),
    )
    .expect_err("annotated mismatch must fail");
    assert_eq!(errors[0].line, 2);
}

#[test]
fn preserves_unknown_runtime_values() {
    check("local value: number = love.timer.getTime()\nprint(value)\n")
        .expect("an external runtime value cannot be rejected by inference alone");
}

#[test]
fn checks_annotated_operator_overload_argument() {
    let errors = check(
        r#"
class Vector is
    public is
        static function new(): Vector
            return {}
        end
        function operator+(other: Vector): Vector
            return self
        end
    end
end
local value = Vector.new()
local invalid = value + 1
"#,
    )
    .expect_err("wrong overload argument must fail");
    assert!(
        errors
            .iter()
            .any(|error| error.contains("operator '+' for 'Vector' expects Vector, got number"))
    );
}

const DOG_MODULE: &str = r#"
class Dog is
    public is
        name:string = "Pochi"
        age = 3
        function bark(): string
            return "wan"
        end
    end
end

local module = {}
module = { dog = Dog.new() }
local clsdef = module
"#;

fn check_with_dog_module(tail: &str) -> Result<(), Vec<String>> {
    check(&format!("{DOG_MODULE}{tail}"))
}

#[test]
fn infers_table_shape_through_reassignment_and_alias() {
    check_with_dog_module("local n: string = clsdef.dog.name\nlocal a: number = clsdef.dog.age\n")
        .expect("field types follow the module table");
    let errors = check_with_dog_module("local n: number = clsdef.dog.name\n")
        .expect_err("string field assigned to number must fail");
    assert!(
        errors
            .iter()
            .any(|error| error.contains("cannot assign string to 'n: number'"))
    );
}

#[test]
fn infers_method_return_type_from_annotation() {
    let errors = check_with_dog_module("local n: number = clsdef.dog.bark()\n")
        .expect_err("annotated return type must be used");
    assert!(
        errors
            .iter()
            .any(|error| error.contains("cannot assign string to 'n: number'"))
    );
}

#[test]
fn shape_tracks_fields_added_after_creation() {
    check("local mod = {}\nmod.title = 'x'\nfunction mod.run() end\nlocal t: string = mod.title\n")
        .expect("assigned field type is tracked");
    let errors = check("local mod = {}\nmod.title = 'x'\nlocal t: number = mod.title\n")
        .expect_err("assigned field type is tracked");
    assert!(errors.iter().any(|error| error.contains("cannot assign string")));
}

#[test]
fn unknown_shape_members_stay_unknown() {
    check("local mod = { a = 1 }\nlocal v: number = mod.missing\nlocal w: string = mod.a\n")
        .expect_err("only the known field w is wrong");
    check("local mod = { a = 1 }\nlocal v: number = mod.missing\n")
        .expect("unknown member must not be rejected");
}
