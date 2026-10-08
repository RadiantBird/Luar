use luar_rs::completion::CompletionItem;
use luar_rs::navigation::{SemanticToken, semantic_tokens};
use luar_rs::{CompileOptions, Target, complete_source_with_options, compile_source_with_options};
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
        let path = std::env::temp_dir().join(format!("luar-stdlib-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn options(&self, target: Target) -> CompileOptions {
        CompileOptions {
            target,
            source_path: Some(self.path.join("main.luar")),
        }
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
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

fn compile(source: &str) -> Result<String, Vec<String>> {
    compile_for(Target::Luau, source)
}

fn assert_error(errors: &[String], expected: &str) {
    assert!(
        errors.iter().any(|error| error.contains(expected)),
        "expected an error containing {expected:?}, got {errors:#?}"
    );
}

// ─── 戻り値の推論 ────────────────────────────────────────────────────────────

#[test]
fn standard_function_return_types_are_inferred() {
    let errors = compile("local n = math.floor(1.5)\nlocal s: string = n\n").unwrap_err();
    assert_error(&errors, "cannot assign number to 's: string'");
    let errors = compile("local s = string.format(\"%d\", 1)\nlocal n: number = s\n").unwrap_err();
    assert_error(&errors, "cannot assign string to 'n: number'");
    let errors = compile("local t = type(1)\nlocal n: number = t\n").unwrap_err();
    assert_error(&errors, "cannot assign string to 'n: number'");
}

#[test]
fn tonumber_is_optional_until_narrowed() {
    let errors = compile("local n = tonumber(\"1\")\nlocal m: number = n\n").unwrap_err();
    assert_error(&errors, "cannot assign number? to 'm: number'");
    compile("local n = tonumber(\"1\") or 0\nlocal m: number = n\n").expect("`or` removes nil");
}

#[test]
fn length_operator_is_a_number() {
    let errors = compile("local t = {1, 2}\nlocal s: string = #t\n").unwrap_err();
    assert_error(&errors, "cannot assign number to 's: string'");
}

#[test]
fn methods_on_strings_use_the_string_library() {
    compile("local s = (\"x\"):upper()\nlocal t: string = s\n").expect("upper returns string");
    let errors = compile("local n: number = (\"x\"):rep(3)\n").unwrap_err();
    assert_error(&errors, "cannot assign string to 'n: number'");
    let errors = compile("local s = (\"x\"):rep(\"a\")\n").unwrap_err();
    assert_error(&errors, "argument 1 of 'rep' expects number, got string");
}

#[test]
fn unannotated_function_return_types_are_inferred() {
    let errors = compile("local function one()\n    return 1\nend\nlocal s: string = one()\n")
        .unwrap_err();
    assert_error(&errors, "cannot assign number to 's: string'");
    let errors = compile("function name()\n    return \"a\"\nend\nlocal n: number = name()\n")
        .unwrap_err();
    assert_error(&errors, "cannot assign string to 'n: number'");
}

#[test]
fn inferred_returns_follow_every_return_statement() {
    compile(
        "function mixed(x)\n    if x then\n        return 1\n    end\n    return \"a\"\nend\nlocal n: number = mixed(true)\n",
    )
    .expect("incompatible returns make the type unknown");
    let errors = compile(
        "function maybe(x)\n    if x then\n        return 1\n    end\n    return nil\nend\nlocal n: number = maybe(true)\n",
    )
    .unwrap_err();
    assert_error(&errors, "cannot assign number? to 'n: number'");
    let errors = compile("function nothing()\nend\nlocal n: number = nothing()\n").unwrap_err();
    assert_error(&errors, "cannot assign nil to 'n: number'");
}

#[test]
fn annotated_return_type_wins_over_the_body() {
    let errors = compile("function f(): string\n    return \"a\"\nend\nlocal n: number = f()\n")
        .unwrap_err();
    assert_error(&errors, "cannot assign string to 'n: number'");
}

#[test]
fn ordinary_function_arguments_are_still_not_checked() {
    compile("function add(a: number, b: number): number\n    return a + b\nend\nlocal x = add(\"x\", {})\n")
        .expect("only template/declare/standard functions check their arguments");
}

// ─── 引数の検査 ──────────────────────────────────────────────────────────────

#[test]
fn standard_function_arguments_are_checked() {
    let errors = compile("local n = math.floor(\"x\")\n").unwrap_err();
    assert_error(&errors, "argument 1 of 'floor' expects number, got string");
    let errors = compile("local n = math.floor()\n").unwrap_err();
    assert_error(&errors, "function 'floor' expects 1 argument(s), got 0");
    let errors = compile("local s = string.rep(\"x\")\n").unwrap_err();
    assert_error(&errors, "function 'rep' expects at least 2 argument(s), got 1");
    let errors = compile("local n = tostring(1, 2)\n").unwrap_err();
    assert_error(&errors, "function 'tostring' expects at most 1 argument(s), got 2");
}

#[test]
fn variadic_standard_functions_accept_any_number_of_arguments() {
    compile("print(1, \"a\", {}, nil)\nlocal s = string.format(\"%d %s\", 1, \"x\", 3)\nlocal m = math.max(1, 2, 3)\n")
        .expect("variadic arguments");
    let errors = compile("local m = math.max()\n").unwrap_err();
    assert_error(&errors, "function 'max' expects at least 1 argument(s), got 0");
    let errors = compile("local m = math.max(1, \"a\")\n").unwrap_err();
    assert_error(&errors, "argument 2 of 'max' expects number, got string");
}

#[test]
fn unknown_values_pass_every_standard_function_check() {
    compile("local n = math.floor(love.timer.getTime())\nlocal s = tostring(love)\n")
        .expect("external runtime values are never rejected");
}

#[test]
fn shadowing_a_standard_name_disables_its_signature() {
    compile("local print = function(a, b) end\nprint(1, 2, 3)\nlocal math = { floor = 1 }\nmath.floor = 2\n")
        .expect("user declarations override the standard library");
}

#[test]
fn standard_libraries_differ_between_targets() {
    let source = "local x: number = math.tointeger(1)\n";
    let errors = compile_for(Target::Lua54, source).unwrap_err();
    assert_error(&errors, "cannot assign number? to 'x: number'");
    compile_for(Target::Luau, source).expect("Luau has no math.tointeger");

    compile_for(Target::Luau, "local x: number = math.clamp(1, 0, 2)\n").expect("Luau math.clamp");
    let errors = compile_for(Target::Luau, "local y = math.clamp(\"a\", 0, 2)\n").unwrap_err();
    assert_error(&errors, "argument 1 of 'clamp' expects number, got string");
}

// ─── 色分け ─────────────────────────────────────────────────────────────────

fn tokens(target: Target, source: &str) -> Vec<SemanticToken> {
    let project = TempProject::new();
    semantic_tokens(source, &project.options(target))
}

fn token_at(tokens: &[SemanticToken], line: usize, column: usize) -> (String, Vec<String>) {
    let token = tokens
        .iter()
        .find(|token| token.line == line && token.column == column)
        .unwrap_or_else(|| panic!("no token at {line}:{column} in {tokens:#?}"));
    (token.kind.clone(), token.modifiers.clone())
}

#[test]
fn standard_names_are_highlighted_as_default_library() {
    let tokens = tokens(
        Target::Luau,
        "print(math.floor(math.pi))\nlocal t = table.concat({}, \",\")\nlocal v = _VERSION\n",
    );
    assert_eq!(token_at(&tokens, 0, 0), ("function".into(), vec!["defaultLibrary".into()]));
    assert_eq!(token_at(&tokens, 0, 6), ("namespace".into(), vec!["defaultLibrary".into()]));
    assert_eq!(token_at(&tokens, 0, 11), ("function".into(), vec!["defaultLibrary".into()]));
    assert_eq!(token_at(&tokens, 0, 17), ("namespace".into(), vec!["defaultLibrary".into()]));
    assert_eq!(
        token_at(&tokens, 0, 22),
        ("variable".into(), vec!["readonly".into(), "defaultLibrary".into()])
    );
    assert_eq!(token_at(&tokens, 1, 10), ("namespace".into(), vec!["defaultLibrary".into()]));
    assert_eq!(token_at(&tokens, 1, 16), ("function".into(), vec!["defaultLibrary".into()]));
    assert_eq!(
        token_at(&tokens, 2, 10),
        ("variable".into(), vec!["readonly".into(), "defaultLibrary".into()])
    );
}

#[test]
fn local_declarations_are_not_default_library() {
    let tokens = tokens(Target::Luau, "local print = 1\nprint = 2\nlocal t = {}\nt.floor = 1\n");
    assert_eq!(token_at(&tokens, 1, 0), ("variable".into(), vec![]));
    assert!(!token_at(&tokens, 3, 2).1.contains(&"defaultLibrary".to_string()));
}

#[test]
fn highlighting_follows_the_target() {
    let luau = tokens(Target::Luau, "task.wait(1)\n");
    assert_eq!(token_at(&luau, 0, 0), ("namespace".into(), vec!["defaultLibrary".into()]));
    let lua = tokens(Target::Lua54, "task.wait(1)\n");
    assert!(!token_at(&lua, 0, 0).1.contains(&"defaultLibrary".to_string()));
}

// ─── 補完 ───────────────────────────────────────────────────────────────────

#[test]
fn namespace_members_are_offered_with_their_kinds() {
    let project = TempProject::new();
    let source = "math.";
    let items: Vec<CompletionItem> = complete_source_with_options(
        source,
        source.encode_utf16().count(),
        &project.options(Target::Luau),
    )
    .expect("completion should work");
    let floor = items.iter().find(|item| item.label == "floor").expect("floor");
    assert_eq!(floor.kind, "function");
    assert!(items.iter().any(|item| item.label == "pi"));
    assert!(items.iter().any(|item| item.label == "clamp"));
}
