use luar_rs::{CompileOptions, Target, compile_source_with_options};

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

fn luau(source: &str) -> String {
    compile_for(Target::Luau, source).unwrap_or_else(|errors| panic!("{errors:?}"))
}

fn lua54(source: &str) -> String {
    compile_for(Target::Lua54, source).unwrap_or_else(|errors| panic!("{errors:?}"))
}

fn assert_error(source: &str, expected: &str) {
    for target in [Target::Luau, Target::Lua54] {
        let errors = compile_for(target, source).expect_err("expected an error");
        assert!(
            errors.iter().any(|error| error.contains(expected)),
            "expected '{expected}' in {errors:?}"
        );
    }
}

const SIGNAL: &str = "class Signal is\n    public is\n        function free()\n            print(\"bye\")\n        end\n        function fire()\n        end\n    end\nend\n";

#[test]
fn using_calls_free_when_the_scope_ends_in_luau() {
    let output = luau(&format!(
        "{SIGNAL}function run()\n    using s = Signal.new()\n    s.fire()\nend\n"
    ));
    assert!(output.contains("const s = Signal.new()"), "{output}");
    assert!(output.contains("table.pack(pcall(function()"), "{output}");
    assert!(output.contains("if s ~= nil then s:free() end"), "{output}");
    assert!(output.contains("error("), "errors are rethrown: {output}");
    let body = output.split("function run()").nth(1).unwrap();
    assert!(body.find("s:fire()").unwrap() < body.find("s:free()").unwrap(), "{output}");
}

#[test]
fn using_uses_a_close_variable_in_lua54() {
    let output = lua54(&format!(
        "{SIGNAL}function run()\n    using s = Signal.new()\n    s.fire()\nend\n"
    ));
    assert!(output.contains("local s <close> = Signal.new()"), "{output}");
    assert!(output.contains("Signal.__close = function(self) self:free() end"), "{output}");
    assert!(!output.contains("pcall"), "{output}");
}

#[test]
fn return_break_and_continue_leave_the_closure_through_a_status_code() {
    let output = luau(&format!(
        "{SIGNAL}function run()\n    for i = 1, 3 do\n        using s = Signal.new()\n        if i == 1 then\n            continue\n        end\n        if i == 2 then\n            break\n        end\n        return i\n    end\nend\n"
    ));
    assert!(output.contains("return 3"), "continue: {output}");
    assert!(output.contains("return 2"), "break: {output}");
    assert!(output.contains("return 1, i"), "return: {output}");
    assert!(output.contains("return table.unpack("), "{output}");
    assert!(output.contains("[2] == 2 then\n            break\n"), "break is relayed to the loop: {output}");
    assert!(output.contains("[2] == 3 then\n            continue\n"), "{output}");
}

#[test]
fn loops_inside_the_scope_keep_their_own_break() {
    let output = luau(&format!(
        "{SIGNAL}function run()\n    using s = Signal.new()\n    for i = 1, 3 do\n        if i == 2 then\n            break\n        end\n    end\nend\n"
    ));
    assert!(!output.contains("return 2"), "{output}");
    assert!(output.contains("break\n"), "{output}");
}

#[test]
fn nested_using_frees_in_reverse_order() {
    let output = luau(&format!(
        "{SIGNAL}function run()\n    using a = Signal.new()\n    using b = Signal.new()\n    return 1\nend\n"
    ));
    let a_free = output.find("a:free()").unwrap();
    let b_free = output.find("b:free()").unwrap();
    assert!(b_free < a_free, "{output}");
    // 内側の return は外側のクロージャへも中継される。
    assert!(output.contains("return 1, table.unpack("), "{output}");
}

#[test]
fn varargs_are_forwarded_into_the_closure_only_when_used() {
    let used = luau(&format!(
        "{SIGNAL}function run(...)\n    using s = Signal.new()\n    print(...)\nend\n"
    ));
    assert!(used.contains("pcall(function(...)"), "{used}");
    assert!(used.contains("end, ...))"), "{used}");
    let unused = luau(&format!(
        "{SIGNAL}function run(...)\n    using s = Signal.new()\n    print(1)\nend\n"
    ));
    assert!(unused.contains("pcall(function()"), "{unused}");
}

#[test]
fn a_return_inside_a_nested_function_is_not_rewritten() {
    let output = luau(&format!(
        "{SIGNAL}function run()\n    using s = Signal.new()\n    local f = function()\n        return 5\n    end\n    return f()\nend\n"
    ));
    assert!(output.contains("return 5\n"), "{output}");
    assert!(output.contains("return 1, f()"), "{output}");
}

#[test]
fn free_is_idempotent_and_chains_to_the_parent() {
    let output = luau(
        "class Base is\n    public is\n        function free()\n            print(\"base\")\n        end\n    end\nend\nclass Child is Base\n    public is\n        function free() override\n            print(\"child\")\n        end\n    end\nend\n",
    );
    assert!(output.contains("function Base.__free(self)"), "{output}");
    assert!(output.contains("if rawget(self, \"__freed\") then return end"), "{output}");
    assert!(output.contains("self.__freed = true"), "{output}");
    let child = output.split("function Child.__free(self)").nth(1).unwrap();
    assert!(child.find("print(\"child\")").unwrap() < child.find("Base.__free(self)").unwrap(), "{output}");
}

#[test]
fn a_parent_without_free_is_not_called() {
    let output = luau(
        "class Base is\n    public is\n        function ping()\n        end\n    end\nend\nclass Child is Base\n    public is\n        function free()\n            print(\"child\")\n        end\n    end\nend\n",
    );
    assert!(!output.contains("Base.__free"), "{output}");
}

#[test]
fn lua54_gives_inheriting_classes_their_own_close_metamethod() {
    let output = lua54(
        "class Base is\n    public is\n        function free()\n        end\n    end\nend\nclass Child is Base\n    public is\n        function ping()\n        end\n    end\nend\n",
    );
    assert!(output.contains("Base.__close = "), "{output}");
    assert!(output.contains("Child.__close = "), "{output}");
}

#[test]
fn classes_without_free_are_unchanged() {
    let output = luau("class Plain is\n    public is\n        function ping()\n        end\n    end\nend\n");
    assert!(!output.contains("__free"), "{output}");
    assert!(!output.contains("__freed"), "{output}");
    let output = lua54("class Plain is\n    public is\n        function ping()\n        end\n    end\nend\n");
    assert!(!output.contains("__close"), "{output}");
}

#[test]
fn using_requires_a_class_with_free() {
    assert_error("local x = 1\nusing y = 5\n", "requires an instance of a class");
    assert_error(
        "class Plain is\n    public is\n        function ping()\n        end\n    end\nend\nusing p = Plain.new()\n",
        "has none",
    );
}

#[test]
fn a_using_variable_cannot_be_reassigned() {
    assert_error(
        &format!("{SIGNAL}using s = Signal.new()\ns = Signal.new()\n"),
        "const binding",
    );
}

#[test]
fn using_needs_an_initializer_and_one_value() {
    assert_error("using s: Signal\n", "initializer");
    assert_error(&format!("{SIGNAL}using s = Signal.new(), 2\n"), "exactly one value");
}

#[test]
fn goto_and_using_cannot_share_a_function() {
    assert_error(
        &format!("{SIGNAL}function run()\n    using s = Signal.new()\n    goto done\n    ::done::\nend\n"),
        "cannot be used in a function that has 'using'",
    );
}

#[test]
fn using_is_rejected_directly_inside_repeat_until() {
    assert_error(
        &format!("{SIGNAL}function run()\n    repeat\n        using s = Signal.new()\n    until true\nend\n"),
        "repeat-until",
    );
}

#[test]
fn using_is_still_usable_as_an_ordinary_name() {
    let output = luau("local using = 1\nusing = using + 1\nprint(using)\n");
    assert!(output.contains("using = using + 1"), "{output}");
}
