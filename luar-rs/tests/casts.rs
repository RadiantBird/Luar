use luar_rs::{CompileOptions, Target, compile_source, compile_source_with_options};

fn compile(source: &str) -> Result<String, Vec<String>> {
    compile_source(source, None)
}

fn compile_for(target: Target, source: &str) -> Result<String, Vec<String>> {
    compile_source_with_options(
        source,
        &CompileOptions {
            target,
            source_path: None,
        },
    )
    .map_err(|errors| errors.into_iter().map(|error| error.message).collect())
}

fn assert_error(errors: &[String], expected: &str) {
    assert!(
        errors.iter().any(|error| error.contains(expected)),
        "expected an error containing {expected:?}, got {errors:#?}"
    );
}

const ANIMALS: &str = r#"
class Animal is
    public is
        static function new(): Animal
            return {}
        end
    end
end
class Dog is Animal
    public is
        name = "dog"
    end
end
class Rock is
    public is
        static function new(): Rock
            return {}
        end
    end
end
"#;

#[test]
fn cast_fixes_the_type_of_an_optional_value() {
    compile("local a: string = \"2\"\nlocal b = tonumber(a) :: number\nlocal c: number = b\n")
        .expect("number? narrowed to number");
    let errors =
        compile("local a = \"2\"\nlocal b = tonumber(a) :: number\nlocal s: string = b\n").unwrap_err();
    assert_error(&errors, "cannot assign number to 's: string'");
}

#[test]
fn cast_between_unrelated_primitives_is_rejected() {
    let errors = compile("local s = 1 :: string\n").unwrap_err();
    assert_error(&errors, "cannot cast number to string");
    let errors = compile("local a = \"x\"\nlocal n = a :: number\n").unwrap_err();
    assert_error(&errors, "cannot cast string to number");
}

#[test]
fn cast_is_allowed_for_related_types_only() {
    compile(&format!(
        "{ANIMALS}local a = Animal.new()\nlocal d = a :: Dog\nlocal up = Dog.new() :: Animal\n"
    ))
    .expect("down and up casts along the inheritance chain");
    let errors = compile(&format!("{ANIMALS}local d = Dog.new() :: Rock\n")).unwrap_err();
    assert_error(&errors, "to Rock");
}

#[test]
fn cast_to_a_class_keeps_the_class_for_member_checks() {
    compile(&format!(
        "{ANIMALS}local t = setmetatable({{}}, {{}}) :: Dog\nlocal d: Dog = t\n"
    ))
    .expect("a table can be treated as a class instance");
}

#[test]
fn cast_of_unknown_values_is_always_allowed() {
    compile("local v = love.timer.getTime() :: number\nlocal n: number = v\n")
        .expect("unknown values can be cast to any type");
}

#[test]
fn labels_are_not_mistaken_for_casts() {
    let output = compile_for(
        Target::Lua54,
        "function main()
    for i = 1, 3 do
        if i == 2 then goto skip end
        print(i :: number)
    end
    ::skip::
    print(\"done\")
end
",
    )
    .expect("label next to a cast");
    assert!(output.contains("::skip::"), "{output}");
    assert!(output.contains("-- cast: i :: number"), "{output}");
}

#[test]
fn cast_binds_tighter_than_binary_operators() {
    let output = compile("local a = 1\nlocal b = 2\nlocal c = a + b :: number\n").unwrap();
    assert!(output.contains("-- cast: b :: number\nlocal c = a + b"), "{output}");
    let output = compile("local a = 1\nlocal b = 2\nlocal c = (a + b) :: number * 2\n").unwrap();
    assert!(output.contains("-- cast: (a + b) :: number"), "{output}");
    assert!(output.contains("local c = (a + b) * 2"), "{output}");
    compile("local x = 1\nlocal y = -x :: number\n").expect("unary minus applies to the cast");
}

#[test]
fn cast_type_errors_in_the_target_are_reported() {
    let errors = compile("local x = 1\nlocal y = x :: number<string>\n").unwrap_err();
    assert_error(&errors, "type 'number' does not take type arguments");
}

#[test]
fn casts_are_erased_and_commented_before_their_statement() {
    for target in [Target::Luau, Target::Lua54] {
        let output = compile_for(
            target,
            "local a = \"2\"\nlocal b = tonumber(a) :: number\nif (b :: number) > 1 then\n    print(b :: number)\nend\n",
        )
        .unwrap();
        assert!(!output.contains("local b = tonumber(a) ::"), "{output}");
        assert!(output.contains("-- cast: tonumber(a) :: number\nlocal b = tonumber(a)"), "{output}");
        assert!(output.contains("-- cast: b :: number\nif b > 1 then"), "{output}");
        assert!(output.contains("    -- cast: b :: number\n    print(b)"), "{output}");
    }
}

#[test]
fn chained_casts_need_parentheses() {
    compile("local x = 1\nlocal y = (x :: any) :: number\n").expect("parenthesized chain");
}
