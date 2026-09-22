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
    let errors = check("local title: string = 42\n").expect_err("annotated mismatch must fail");
    assert!(
        errors
            .iter()
            .any(|error| error.contains("cannot assign number to 'title: string'"))
    );
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
