use luar_rs::{
    CompileOptions, Target, analyze_source_with_options, check_source_with_options,
    compile_source_with_options,
};

fn options(target: Target) -> CompileOptions {
    CompileOptions {
        target,
        source_path: None,
    }
}

#[test]
fn simple_if_expression_uses_native_luau_form() {
    let output = compile_source_with_options(
        "local x = if cond then 1 else 2 end",
        &options(Target::Luau),
    )
    .unwrap();
    assert!(output.contains("local x = if cond then 1 else 2"));
}

#[test]
fn simple_elseif_expression_uses_luau_elseif() {
    let output = compile_source_with_options(
        "local x = if first then 1 elseif second then 2 else 3 end",
        &options(Target::Luau),
    )
    .unwrap();
    assert!(output.contains("if first then 1 elseif second then 2 else 3"));
    assert!(!output.contains("then if second"));
}

#[test]
fn block_if_expression_lowers_to_a_temporary() {
    let output = compile_source_with_options(
        "local x = if cond then\nfoo()\n123\nelse\nbar()\n456\nend",
        &options(Target::Luau),
    )
    .unwrap();
    assert!(output.contains("local __luar_if_"));
    assert!(output.contains("foo()"));
    assert!(output.contains("= 123"));
    assert!(output.contains("bar()"));
    assert!(output.contains("= 456"));
    assert!(output.contains("local x = __luar_if_"));
}

#[test]
fn elseif_and_nested_if_expressions_are_accepted() {
    let output = compile_source_with_options(
        "local x = if n > 10 then\nprint(\"big\")\n\"big\"\nelseif n > 5 then\nprint(\"medium\")\n\"medium\"\nelse\nif inner then\n\"small-a\"\nelse\n\"small-b\"\nend\nend",
        &options(Target::Luau),
    )
    .unwrap();
    assert!(output.contains("n > 10"));
    assert!(output.contains("n > 5"));
    assert!(output.contains("small-a"));
}

#[test]
fn if_expression_is_valid_in_arguments_binary_expressions_and_return() {
    let output = compile_source_with_options(
        "function f()\nfoo(10, if cond then 20 else 30 end)\nreturn 100 + if cond then calculate() 1 else 2 end\nend",
        &options(Target::Luau),
    )
    .unwrap();
    assert!(output.contains("foo(10, (if cond then 20 else 30"));
    assert!(output.contains("calculate()"));
    assert!(output.contains("return"));
}

#[test]
fn branch_locals_are_available_to_the_branch_result_only() {
    let output = compile_source_with_options(
        "local x = if cond then\nlocal value = 41\nvalue\nelse\n0\nend",
        &options(Target::Lua54),
    )
    .unwrap();
    assert!(output.contains("local value = 41"));
    assert!(output.contains("= value"));
}

#[test]
fn lua54_always_uses_statement_lowering_and_preserves_false_and_nil() {
    let output = compile_source_with_options(
        "local a = if cond then false else true end\nlocal b = if cond then nil else nil end",
        &options(Target::Lua54),
    )
    .unwrap();
    assert!(output.matches("local __luar_if_").count() >= 2);
    assert!(output.contains("= false"));
    assert!(output.contains("= nil"));
    assert!(!output.contains(" and "));
}

#[test]
fn incompatible_branch_types_are_rejected() {
    let errors = check_source_with_options(
        "local x: number = if cond then 1 else \"hello\" end",
        &options(Target::Luau),
    )
    .unwrap_err();
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("if expression branches have incompatible result types")
    }));
}

#[test]
fn nil_and_value_branches_use_the_existing_optional_type_rules() {
    compile_source_with_options(
        "local value: number? = if cond then 1 else nil end",
        &options(Target::Luau),
    )
    .unwrap();

    let errors = check_source_with_options(
        "local value: number = if cond then 1 else nil end",
        &options(Target::Luau),
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("cannot assign number?"))
    );
}

#[test]
fn missing_else_and_missing_branch_result_are_rejected() {
    let missing_else =
        check_source_with_options("local x = if cond then 1 end", &options(Target::Luau))
            .unwrap_err();
    assert!(
        missing_else
            .iter()
            .any(|error| error.message.contains("requires an else branch"))
    );

    let missing_result = check_source_with_options(
        "local x = if cond then print(\"hello\") else 123 end",
        &options(Target::Luau),
    )
    .unwrap_err();
    assert!(
        missing_result
            .iter()
            .any(|error| error.message.contains("must end with a value expression"))
    );
}

#[test]
fn generated_temporaries_avoid_user_names() {
    let output = compile_source_with_options(
        "local __luar_if_0 = 9\nlocal x = if cond then foo() 1 else 2 end",
        &options(Target::Lua54),
    )
    .unwrap();
    assert!(output.contains("local __luar_if_1"));
    assert!(!output.contains("local __luar_if_0\nif cond"));
}

#[test]
fn branch_side_effects_stay_before_their_result_assignment() {
    let output = compile_source_with_options(
        "local x = if cond then first() 1 else second() 2 end",
        &options(Target::Lua54),
    )
    .unwrap();
    let first = output.find("first()").unwrap();
    let first_result = output[first..].find("= 1").unwrap() + first;
    let second = output.find("second()").unwrap();
    let second_result = output[second..].find("= 2").unwrap() + second;
    assert!(first < first_result);
    assert!(second < second_result);
}

#[test]
fn if_binding_lowers_to_a_scoped_local() {
    let output = compile_source_with_options(
        "if c := getSomething() then\nprint(c)\nelse\nprint(\"not found\")\nend",
        &options(Target::Luau),
    )
    .unwrap();
    assert!(output.contains("do\n    local c = getSomething()"));
    assert!(output.contains("if c then"));
    assert_eq!(output.matches("getSomething()").count(), 1);
}

#[test]
fn elseif_binding_is_short_circuiting_and_scoped() {
    let output = compile_source_with_options(
        "if a := getA() then\nprint(a)\nelseif b := getB() then\nprint(b)\nelse\nprint(\"nothing\")\nend",
        &options(Target::Lua54),
    )
    .unwrap();
    assert!(output.contains("local a = getA()"));
    assert!(output.contains("local b = getB()"));
    assert!(output.contains("if a then"));
    assert!(output.contains("if b then"));
    assert!(!output.contains("elseif b"));
}

#[test]
fn while_binding_re_evaluates_inside_the_loop_and_supports_continue() {
    let output = compile_source_with_options(
        "while line := file:ReadLine() do\nif line == \"skip\" then\ncontinue\nend\nprint(line)\nend",
        &options(Target::Luau),
    )
    .unwrap();
    assert!(output.contains("while true do"));
    assert!(output.contains("local line = file:ReadLine()"));
    assert!(output.contains("if not line then break end"));
    assert!(output.contains("continue"));
}

#[test]
fn binding_rhs_is_evaluated_once_per_condition() {
    let output = compile_source_with_options(
        "local outer = if value := sideEffect() then\nvalue\nelse\n0\nend",
        &options(Target::Lua54),
    )
    .unwrap();
    assert_eq!(output.matches("sideEffect()").count(), 1);
    assert!(output.contains("local value = sideEffect()"));
}

#[test]
fn binding_shadows_but_does_not_replace_an_outer_local() {
    let output = compile_source_with_options(
        "local c = \"outer\"\nif c := getSomething() then\nprint(c)\nend\nprint(c)",
        &options(Target::Luau),
    )
    .unwrap();
    assert!(output.contains("local c = \"outer\""));
    assert_eq!(output.matches("local c =").count(), 2);
}

#[test]
fn binding_is_not_visible_in_else_or_after_if() {
    let analysis = analyze_source_with_options(
        "if c := getSomething() then\nprint(c)\nelse\nprint(c)\nend\nprint(c)",
        &options(Target::Luau),
    )
    .unwrap();
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message.contains("unknown global 'c'"))
            .count(),
        2
    );
}

#[test]
fn binding_is_not_visible_in_a_following_elseif_condition() {
    let analysis = analyze_source_with_options(
        "if a := getA() then\nprint(a)\nelseif a.Enabled then\nprint(a)\nend",
        &options(Target::Luau),
    )
    .unwrap();
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unknown global 'a'"))
    );
}

#[test]
fn optional_binding_is_refined_in_the_truthy_branch() {
    compile_source_with_options(
        "local maybe: number? = getNumber()\nif value := maybe then\nlocal result: number = value\nend",
        &options(Target::Luau),
    )
    .unwrap();
}

#[test]
fn if_expression_binding_uses_statement_lowering_for_both_targets() {
    let source = "local result = if c := getSomething() then\nprepare(c)\nc.Value\nelse\n0\nend";
    for target in [Target::Luau, Target::Lua54] {
        let output = compile_source_with_options(source, &options(target)).unwrap();
        assert!(output.contains("local c = getSomething()"));
        assert!(output.contains("if c then"));
        assert!(!output.contains(":="));
        assert_eq!(output.matches("getSomething()").count(), 1);
    }
}

#[test]
fn false_nil_and_zero_follow_lua_truthiness_without_rewriting() {
    for rhs in ["false", "nil", "0"] {
        let output = compile_source_with_options(
            &format!("if value := {rhs} then\nprint(value)\nend"),
            &options(Target::Lua54),
        )
        .unwrap();
        assert!(output.contains(&format!("local value = {rhs}")));
        assert!(output.contains("if value then"));
    }
}

#[test]
fn invalid_binding_positions_have_actionable_diagnostics() {
    let member = check_source_with_options(
        "if object.field := getSomething() then\nend",
        &options(Target::Luau),
    )
    .unwrap_err();
    assert!(member.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("left side of ':=' must be a bare identifier")
    }));

    let outside = check_source_with_options(
        "local value = other := getSomething()",
        &options(Target::Luau),
    )
    .unwrap_err();
    assert!(outside.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("only allowed in conditional contexts")
    }));

    let multiple = check_source_with_options(
        "if a, b := getSomething() then\nend",
        &options(Target::Luau),
    )
    .unwrap_err();
    assert!(multiple.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("left side of ':=' must be a single bare identifier")
    }));
}

#[test]
fn binding_temporary_names_remain_hygienic() {
    let output = compile_source_with_options(
        "local __luar_if_0 = 1\nlocal result = if c := getSomething() then\nc\nelse\n0\nend",
        &options(Target::Lua54),
    )
    .unwrap();
    assert!(output.contains("local __luar_if_1"));
    assert!(!output.contains("local __luar_if_0\n"));
}
