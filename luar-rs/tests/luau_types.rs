use luar_rs::{CompileOptions, Target, compile_source_with_options};

fn compile_for(target: Target, source: &str) -> String {
    compile_source_with_options(
        source,
        &CompileOptions {
            target,
            source_path: None,
        },
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
}

fn luau(source: &str) -> String {
    compile_for(Target::Luau, source)
}

#[test]
fn functions_keep_parameter_and_return_types() {
    let output = luau(
        "template <T>\nfunction first(items: { T }, fallback: T?): T\n    return items[1] or fallback\nend\nlocal function twice(x: number): number\n    return x * 2\nend\nfunction plain(a)\n    return a\nend\n",
    );
    assert!(output.contains("function first<T>(items: { T }, fallback: T?): T"), "{output}");
    assert!(output.contains("local function twice(x: number): number"), "{output}");
    assert!(output.contains("function plain(a)"), "{output}");
    assert!(!output.contains("-- function"), "annotations replace the comments: {output}");
}

#[test]
fn locals_and_consts_keep_their_types() {
    let output = luau("local score: number, name = 1, \"x\"\nconst LIMIT: number | string = 10\nprint(score, name, LIMIT)\n");
    assert!(output.contains("local score: number, name = 1, \"x\""), "{output}");
    assert!(output.contains("const LIMIT: number | string = 10"), "{output}");
}

#[test]
fn type_aliases_are_real_luau_types_and_can_be_referenced() {
    let output = luau(
        "template <T>\nexport type MyTable = { id: number, ref: T }\ntype Name = string\nlocal t: MyTable<number> = { id = 1, ref = 2 }\nlocal n: Name = \"a\"\nprint(t, n)\n",
    );
    assert!(output.contains("export type MyTable<T> = { id: number, ref: T }"), "{output}");
    assert!(output.contains("type Name = string"), "{output}");
    assert!(output.contains("local t: MyTable<number> = "), "{output}");
    assert!(output.contains("local n: Name = \"a\""), "{output}");
}

#[test]
fn class_types_that_luau_cannot_see_become_any() {
    let output = luau(
        "class Dog is\n    public is\n        name = \"d\"\n    end\nend\nfunction feed(d: Dog, more: { Dog }, maybe: Dog?): Dog\n    return d\nend\n",
    );
    assert!(output.contains("function feed(d: any, more: { any }, maybe: any?): any"), "{output}");
}

#[test]
fn class_methods_are_annotated_and_self_is_not() {
    let output = luau(
        "class Counter is\n    public is\n        value = 0\n        function add(amount: number): number\n            return self.value + amount\n        end\n        static function make(start: number)\n            return start\n        end\n        template <A>\n        function pick(item: A): A\n            return item\n        end\n    end\nend\n",
    );
    assert!(output.contains("function Counter.add(self, amount: number): number"), "{output}");
    assert!(output.contains("function Counter.make(start: number)"), "{output}");
    assert!(output.contains("function Counter.pick<A>(self, item: A): A"), "{output}");
}

#[test]
fn function_types_and_unions_are_written_in_luau_syntax() {
    let output = luau(
        "local handler: (number, string) -> boolean = print\nlocal maybe: (number | string)? = nil\nlocal either: number | (() -> ()) = 1\nprint(handler, maybe, either)\n",
    );
    assert!(output.contains("local handler: (number, string) -> boolean = print"), "{output}");
    assert!(output.contains("local maybe: (number | string)? = nil"), "{output}");
    assert!(output.contains("local either: number | (() -> ()) = 1"), "{output}");
}

#[test]
fn lua54_still_erases_every_annotation() {
    let output = compile_for(
        Target::Lua54,
        "template <T>\nexport type Box = { value: T }\nfunction first(items: { number }, n: number): number\n    return items[n]\nend\nlocal count: number = 1\nprint(first({ 1 }, count))\n",
    );
    assert!(output.contains("function first(items, n)"), "{output}");
    assert!(output.contains("local count = 1"), "{output}");
    assert!(output.contains("-- export type Box<T> = { value: T }"), "{output}");
    let code: Vec<&str> = output
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect();
    assert!(!code.join("\n").contains("number"), "{output}");
}
