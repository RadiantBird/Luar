use luar_rs::{CompileOptions, Severity, Target, analyze_source_with_options, compile_source};
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
            "luar-rs-{}-{}-{id}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
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

    fn definition_bytes(&self, module: &str, source: &[u8]) {
        fs::write(self.path.join(format!("{module}.luard")), source).unwrap();
    }

    fn source(&self, name: &str, source: &str) -> PathBuf {
        let path = self.path.join(name);
        fs::write(&path, source).unwrap();
        path
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn compile_at(source: &str, path: &Path) -> Result<String, Vec<String>> {
    compile_source(source, Some(path))
}

fn assert_error(errors: &[String], expected: &str) {
    assert!(
        errors.iter().any(|error| error.contains(expected)),
        "expected an error containing {expected:?}, got {errors:#?}"
    );
}

#[test]
fn adjacent_definition_rewrites_unqualified_reads_writes_and_calls() {
    let project = TempProject::new();
    project.definition(
        "numbers",
        r#"
-- declarations can contain comments and blank lines
declare answer: number
declare calculate: (number, number) -> number
"#,
    );

    let output = compile_at(
        r#"
import type numbers
print(answer)
answer = calculate(answer, 1)
local record = { answer = answer, [answer] = calculate }
"#,
        &project.source_path(),
    )
    .unwrap();

    assert!(output.contains("print(numbers.answer)"));
    assert!(output.contains("numbers.answer = numbers.calculate(numbers.answer, 1)"));
    assert!(output.contains("{ answer = numbers.answer, [numbers.answer] = numbers.calculate }"));
    assert!(!output.contains("import"));
}

#[test]
fn ambiguity_is_reported_only_when_unqualified_name_is_used() {
    let project = TempProject::new();
    project.definition("first", "declare shared: string");
    project.definition("second", "declare shared: number");

    let unused = compile_at(
        "import type first\nimport type second\nprint(first.shared, second.shared)",
        &project.source_path(),
    )
    .unwrap();
    assert!(unused.contains("print(first.shared, second.shared)"));

    let errors = compile_at(
        "import type first\nimport type second\nprint(shared)",
        &project.source_path(),
    )
    .unwrap_err();
    assert_error(&errors, "ambiguous unqualified name 'shared'");
    assert_error(&errors, "first, second");
}

#[test]
fn globals_unknown_names_and_qualified_names_remain_unqualified() {
    let project = TempProject::new();
    project.definition(
        "engine",
        "declare value: number\ndeclare global workspace: workspace",
    );

    let output = compile_at(
        "import type engine\nprint(engine.value, workspace, unknownGlobal)",
        &project.source_path(),
    )
    .unwrap();
    assert!(output.contains("print(engine.value, workspace, unknownGlobal)"));
    assert!(!output.contains("engine.workspace"));
}

#[test]
fn diagnostics_after_include_use_the_original_main_source_line() {
    let project = TempProject::new();
    project.source(
        "colors.luar",
        "local colors = {}\ncolors.background = { 0, 0, 0 }\nreturn colors\n",
    );
    let main = project.source_path();
    let analysis = analyze_source_with_options(
        "local colors = !include(\"./colors.luar\")\nlovve.graphics.print(colors.background)",
        &CompileOptions {
            target: Target::Luau,
            source_path: Some(main.clone()),
        },
    )
    .expect("unknown globals are warnings, not errors");

    let warning = analysis
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("unknown global 'lovve'"))
        .expect("unknown global warning");
    assert_eq!(warning.severity, Severity::Warning);
    assert_eq!(warning.file, main.display().to_string());
    assert_eq!(warning.line, 2);
    assert_eq!(warning.column, 1);
    assert_eq!(warning.end_column, 6);
}

#[test]
fn lexical_bindings_shadow_module_members() {
    let project = TempProject::new();
    project.definition("module", "declare value: number");

    let output = compile_at(
        r#"
import type module
local value = 1
do
    const value = 2
    print(value)
end
local callback = function(value)
    print(value)
end
for value = 1, 2 do
    print(value)
end
for value in values do
    print(value)
end
"#,
        &project.source_path(),
    )
    .unwrap();

    assert!(!output.contains("module.value"));
}

#[test]
fn class_and_function_names_shadow_module_members() {
    let project = TempProject::new();
    project.definition("module", "declare Worker: string\ndeclare recurse: string");

    let output = compile_at(
        r#"
import type module
class Worker is
    function recurse()
        recurse()
    end
end
const function recurse()
    recurse()
end
"#,
        &project.source_path(),
    )
    .unwrap();

    assert!(!output.contains("module.Worker"));
    assert!(!output.contains("module.recurse"));
}

#[test]
fn definition_and_import_failures_include_actionable_diagnostics() {
    let project = TempProject::new();

    let missing =
        compile_at("import type missing\nprint(value)", &project.source_path()).unwrap_err();
    assert_error(&missing, "cannot read module definition");
    assert_error(&missing, "missing.luard");

    project.definition("bad", "declare okay: number\nthis is not valid");
    let invalid = compile_at("import type bad", &project.source_path()).unwrap_err();
    assert_error(&invalid, "bad.luard:2");
    assert_error(&invalid, "expected 'declare'");

    project.definition("duplicate", "declare value: number\ndeclare value: string");
    let duplicate = compile_at("import type duplicate", &project.source_path()).unwrap_err();
    assert_error(&duplicate, "duplicate.luard:2");
    assert_error(&duplicate, "declared more than once");

    project.definition_bytes("utf8", &[0xff, 0xfe]);
    let utf8 = compile_at("import type utf8", &project.source_path()).unwrap_err();
    assert_error(&utf8, "not valid UTF-8");
}

#[test]
fn definition_function_types_accept_optional_names_and_tuple_returns() {
    let project = TempProject::new();
    project.definition(
        "mod",
        r#"
declare run: () -> ()
declare transform: (string?, number) -> boolean?
declare split: () -> (string, number)
"#,
    );

    let output = compile_at(
        r#"
import type mod
const mod = require("@./mod.luar")
mod.run()
"#,
        &project.source_path(),
    )
    .unwrap();

    assert!(output.contains("const mod = require(\"@./mod.luar\")"));
    assert!(output.contains("mod.run()"));
    assert!(!output.contains("import type"));
}

#[test]
fn top_level_named_functions_compile_with_plain_and_qualified_names() {
    let output = compile_at(
        r#"
local mod = {}
function mod.run()
    print("this is mod")
end
function greet()
    print("hello")
end
return mod
"#,
        Path::new("main.luar"),
    )
    .expect("top-level named functions should compile");

    assert!(output.contains("local mod = {}"));
    assert!(output.contains("function mod.run()"));
    assert!(output.contains("print(\"this is mod\")"));
    assert!(output.contains("function greet()"));
    assert!(output.contains("print(\"hello\")"));
    assert!(output.contains("return mod"));
}

#[test]
fn malformed_definition_function_types_include_path_and_line() {
    let project = TempProject::new();
    project.definition("bad_arrow", "declare run: string -> ()");
    let invalid_arrow = compile_at("import type bad_arrow", &project.source_path()).unwrap_err();
    assert_error(&invalid_arrow, "bad_arrow.luard:1");
    assert_error(
        &invalid_arrow,
        "function type parameters must be enclosed in parentheses",
    );

    project.definition("missing_return", "declare run: () ->");
    let missing_return =
        compile_at("import type missing_return", &project.source_path()).unwrap_err();
    assert_error(&missing_return, "missing_return.luard:1");
    assert_error(&missing_return, "expected identifier");
}

#[test]
fn duplicate_type_imports_legacy_import_and_source_declare_are_rejected() {
    let project = TempProject::new();
    project.definition("module", "declare value: number");

    let duplicate = compile_at(
        "import type module\nimport type module\nprint(value)",
        &project.source_path(),
    )
    .unwrap_err();
    assert_error(&duplicate, "imported more than once");

    let legacy = compile_at("import module", &project.source_path()).unwrap_err();
    assert_error(&legacy, "expected 'type' after 'import'");
    assert_error(&legacy, "import type <module>");

    let no_path = compile_source("import type module\nprint(value)", None).unwrap_err();
    assert_error(&no_path, "require luar_compile_with_path");

    let empty_path =
        compile_source("import type module\nprint(value)", Some(Path::new(""))).unwrap_err();
    assert_error(&empty_path, "require a non-empty source file path");

    let source_declare = compile_source("declare value: number\nprint(value)", None).unwrap_err();
    assert_error(
        &source_declare,
        "'declare' is only allowed in .luard module definition files",
    );
}

#[test]
fn const_declarations_and_const_functions_generate_luau_syntax() {
    let output = compile_source(
        r#"
const first: number, second = 1, 2
const function add(value: number): number
    return first + value
end
local const = "contextual"
print(first, second, add(3), const)
"#,
        None,
    )
    .unwrap();

    assert!(output.contains("const first, second = 1, 2"));
    assert!(output.contains("const function add(value)"));
    assert!(output.contains("return first + value"));
    assert!(output.contains("local const = \"contextual\""));
}

#[test]
fn const_remains_an_identifier_outside_declaration_context() {
    let output = compile_source(
        r#"
local const = function()
    return 1
end
const()
const = function()
    return 2
end
"#,
        None,
    )
    .unwrap();

    assert!(output.contains("const()"));
    assert!(output.contains("const = function()"));
}

#[test]
fn const_requires_initialization_and_cannot_be_reassigned() {
    let parse_error = compile_source("const value", None).unwrap_err();
    assert_error(&parse_error, "const declaration must have an initializer");

    let missing_value = compile_source("const first, second = 1", None).unwrap_err();
    assert_error(
        &missing_value,
        "every const binding must have an initializer",
    );

    let reassigned = compile_source("const value = 1\nvalue = 2", None).unwrap_err();
    assert_error(&reassigned, "cannot assign to const binding 'value'");

    let captured = compile_source(
        r#"
const value = 1
local callback = function()
    value = 2
end
"#,
        None,
    )
    .unwrap_err();
    assert_error(&captured, "cannot assign to const binding 'value'");
}

#[test]
fn final_call_method_call_or_vararg_can_initialize_multiple_const_bindings() {
    let output = compile_source(
        r#"
const first, second = getValues()
const third, fourth = source:getValues()
local callback = function(...)
    const fifth, sixth = ...
    return fifth, sixth
end
"#,
        None,
    )
    .unwrap();

    assert!(output.contains("const first, second = getValues()"));
    assert!(output.contains("const third, fourth = source:getValues()"));
    assert!(output.contains("const fifth, sixth = ..."));
}

#[test]
fn const_value_mutation_and_inner_scope_shadowing_are_allowed() {
    let output = compile_source(
        r#"
const value = { count = 1 }
value.count = 2
do
    const value = { count = 3 }
    value.count = 4
end
"#,
        None,
    )
    .unwrap();

    assert!(output.contains("value.count = 2"));
    assert!(output.contains("value.count = 4"));
}

#[test]
fn const_redeclaration_in_the_same_scope_is_rejected() {
    let errors = compile_source("const value = 1\nlocal value = 2", None).unwrap_err();
    assert_error(&errors, "binding 'value' is already declared in this scope");

    let errors = compile_source("local value = 1\nconst value = 2", None).unwrap_err();
    assert_error(&errors, "binding 'value' is already declared in this scope");
}

#[test]
fn const_binding_shadows_a_module_member_before_rewrite() {
    let project = TempProject::new();
    project.definition("module", "declare value: number");

    let output = compile_at(
        "import type module\nconst value = 10\nprint(value)",
        &project.source_path(),
    )
    .unwrap();
    assert!(output.contains("print(value)"));
    assert!(!output.contains("module.value"));
}

#[test]
fn include_inlines_the_returned_module_body() {
    let project = TempProject::new();
    project.definition("mod", "declare hogehoge: string\ndeclare run: () -> ()\n");
    project.source(
        "mod.luar",
        r#"local mod = {}
mod.hogehoge = "gepyaaa"

function mod.run()
    print("this is mod, not admin!")
end

return mod
"#,
    );

    let output = compile_at(
        r#"import type mod
local mod = !include("@./mod.luar")
mod.run()
print(mod.hogehoge)"#,
        &project.source_path(),
    )
    .unwrap();

    assert!(!output.contains("!include"));
    assert!(!output.contains("return mod"));
    assert!(!output.contains("import type"));
    assert!(output.contains("local mod = {}"));
    assert!(output.contains("function mod.run()"));
    assert!(output.contains("mod.run()\nprint(mod.hogehoge)"));
}

#[test]
fn include_creates_an_alias_when_the_returned_name_differs() {
    let project = TempProject::new();
    project.source(
        "implementation.luar",
        "local implementation = {}\nreturn implementation\n",
    );

    let output = compile_at(
        "const api = !include(\"./implementation.luar\")",
        &project.source_path(),
    )
    .unwrap();

    assert!(output.contains("local implementation = {}"));
    assert!(output.contains("const api = implementation"));
}

#[test]
fn include_reports_missing_extension_cycle_and_terminal_return_errors() {
    let project = TempProject::new();
    let main = project.source("main.luar", "local mod = !include(\"./missing.luar\")");
    let missing = compile_at("local mod = !include(\"./missing.luar\")", &main).unwrap_err();
    assert_error(&missing, "main.luar:1");
    assert_error(&missing, "cannot read included source");

    let lua_missing = compile_at("local mod = !include(\"./mod.lua\")", &main).unwrap_err();
    assert_error(&lua_missing, "main.luar:1");
    assert_error(&lua_missing, "cannot read included source");

    let quoted_lua_missing = compile_at("local mod = !include('./mod.lua')", &main).unwrap_err();
    assert_error(&quoted_lua_missing, "cannot read included source");

    project.source("without_return.luar", "local mod = {}\n");
    let no_return =
        compile_at("local mod = !include(\"./without_return.luar\")", &main).unwrap_err();
    assert_error(&no_return, "without_return.luar:1");
    assert_error(&no_return, "must end with standalone");

    project.source(
        "first.luar",
        "local first = !include(\"./second.luar\")\nreturn first\n",
    );
    project.source(
        "second.luar",
        "local second = !include(\"./first.luar\")\nreturn second\n",
    );
    let cycle = compile_at("local first = !include(\"./first.luar\")", &main).unwrap_err();
    assert_error(&cycle, "!include cycle detected");
}

#[test]
fn include_requires_a_source_path() {
    let errors = compile_source("local mod = !include(\"./mod.luar\")", None).unwrap_err();
    assert_error(&errors, "require compile_source with a source file path");
}

#[test]
fn include_works_without_import_type_or_definition_file() {
    let project = TempProject::new();
    project.source(
        "clsdef.luar",
        "local module = {}\nclass Dog is\n    public is\n        name:string = \"Pochi\"\n    end\nend\n\nmodule = {\n    dog = Dog.new()\n}\n\nreturn module\n",
    );
    let main = project.source(
        "main.luar",
        "local clsdef = !include(\"./clsdef.luar\")\nprint(clsdef.dog.name)\n",
    );
    let source = fs::read_to_string(&main).unwrap();
    let output = compile_at(&source, &main).expect("include alone must be enough");
    assert!(output.contains("function Dog.new()"));
    assert!(output.contains("self.name = \"Pochi\""));
    assert!(output.contains("print(clsdef.dog.name)"));
}

#[test]
fn import_type_of_an_included_module_needs_no_definition_file() {
    let project = TempProject::new();
    project.source(
        "mod.luar",
        "local mod = {}\nmod.hogehoge = \"gepyaaa\"\nfunction mod.run()\nend\nreturn mod\n",
    );
    let main = project.source(
        "main.luar",
        "import type mod\nlocal mod = !include(\"./mod.luar\")\nmod.run()\nprint(hogehoge)\n",
    );
    let source = fs::read_to_string(&main).unwrap();
    let output = compile_at(&source, &main).expect("included source provides the declarations");
    assert!(output.contains("print(mod.hogehoge)"), "{output}");
}

#[test]
fn import_type_without_definition_or_include_is_still_an_error() {
    let project = TempProject::new();
    let main = project.source("main.luar", "import type nothing\nprint(1)\n");
    let source = fs::read_to_string(&main).unwrap();
    let errors = compile_at(&source, &main).unwrap_err();
    assert_error(&errors, "cannot read module definition");
}

const DOG_INCLUDE: &str = "local module = {}\nclass Dog is\n    public is\n        name:string = \"Pochi\"\n    end\nend\n\nmodule = {\n    dog = Dog.new()\n}\n\nreturn module\n";

#[test]
fn include_renames_names_that_collide_with_the_including_file() {
    let project = TempProject::new();
    project.source("clsdef.luar", DOG_INCLUDE);
    let main = project.source(
        "main.luar",
        "local module = 1\nclass Dog is\nend\nlocal clsdef = !include(\"./clsdef.luar\")\nprint(module, clsdef.dog.name)\n",
    );
    let source = fs::read_to_string(&main).unwrap();
    let output = compile_at(&source, &main).expect("colliding names must be renamed");
    assert!(output.contains("local module__clsdef = {}"), "{output}");
    assert!(output.contains("function Dog__clsdef.new()"), "{output}");
    assert!(output.contains("module__clsdef = { dog = Dog__clsdef.new() }"), "{output}");
    assert!(output.contains("local clsdef = module__clsdef"), "{output}");
    assert!(output.contains("print(module, clsdef.dog.name)"), "{output}");
    assert!(output.contains("local Dog = {}"), "main's own class keeps its name: {output}");
}

#[test]
fn include_does_not_rename_when_nothing_collides() {
    let project = TempProject::new();
    project.source("clsdef.luar", DOG_INCLUDE);
    let main = project.source(
        "main.luar",
        "local clsdef = !include(\"./clsdef.luar\")\nlocal d = Dog.new()\nprint(clsdef.dog.name)\n",
    );
    let source = fs::read_to_string(&main).unwrap();
    let output = compile_at(&source, &main).unwrap();
    assert!(output.contains("local module = {}"), "{output}");
    assert!(output.contains("local Dog = {}"), "{output}");
    assert!(output.contains("local d = Dog.new()"), "using a leaked name is not a collision: {output}");
    assert!(!output.contains("__clsdef"), "{output}");
}

#[test]
fn two_includes_with_the_same_internal_names_do_not_collide() {
    let project = TempProject::new();
    project.source("a.luar", "local module = { id = \"a\" }\nreturn module\n");
    project.source("b.luar", "local module = { id = \"b\" }\nreturn module\n");
    let main = project.source(
        "main.luar",
        "local a = !include(\"./a.luar\")\nlocal b = !include(\"./b.luar\")\nprint(a.id, b.id)\n",
    );
    let source = fs::read_to_string(&main).unwrap();
    let output = compile_at(&source, &main).unwrap();
    assert!(output.contains("local module = { id = \"a\" }"), "{output}");
    assert!(output.contains("local module__b = { id = \"b\" }"), "{output}");
    assert!(output.contains("local b = module__b"), "{output}");
}

#[test]
fn colliding_name_used_in_a_template_string_is_reported() {
    let project = TempProject::new();
    project.source(
        "m.luar",
        "local label = \"x\"\nlocal m = { text = `{label}` }\nreturn m\n",
    );
    let main = project.source(
        "main.luar",
        "local label = 1\nlocal m = !include(\"./m.luar\")\n",
    );
    let source = fs::read_to_string(&main).unwrap();
    let errors = compile_at(&source, &main).unwrap_err();
    assert_error(&errors, "template string");
}

const DOG_LUARD: &str = "declare class Dog\n    name: string\n    function bark(times: number): string\n    static function create(): Dog\nend\ndeclare dog: Dog\ndeclare run: (Dog) -> ()\n";

fn compile_with_dog_luard(tail: &str) -> Result<String, Vec<String>> {
    let project = TempProject::new();
    project.definition("clsdef", DOG_LUARD);
    let main = project.source("main.luar", "import type clsdef\nlocal clsdef = require(\"./clsdef\")\n");
    let source = format!("import type clsdef\nlocal clsdef = require(\"./clsdef\")\n{tail}");
    compile_at(&source, &main)
}

#[test]
fn luard_declare_class_types_module_members() {
    compile_with_dog_luard(
        "local n: string = clsdef.dog.name\nlocal d: Dog = clsdef.dog\nlocal s: string = clsdef.dog.bark(1)\nclsdef.run(d)\nlocal made = Dog.create()\n",
    )
    .expect("declared members should type-check");

    let errors = compile_with_dog_luard("local n: number = clsdef.dog.name\n").unwrap_err();
    assert_error(&errors, "cannot assign string to 'n: number'");
    let errors = compile_with_dog_luard("local n: number = clsdef.dog.bark(1)\n").unwrap_err();
    assert_error(&errors, "cannot assign string to 'n: number'");
}

#[test]
fn luard_declare_class_emits_no_runtime_code() {
    let output = compile_with_dog_luard("local d: Dog = clsdef.dog\n").unwrap();
    // 型注釈は元のシグネチャーのコメントとして残るが、実行コードには現れない。
    assert!(output.contains("-- local d: Dog"), "{output}");
    let code: Vec<&str> = output
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect();
    assert!(!code.join("\n").contains("Dog"), "{output}");
}

#[test]
fn luard_declare_class_errors_are_reported() {
    let project = TempProject::new();
    project.definition("broken", "declare class Dog\n    name: string\n");
    let main = project.source("main.luar", "import type broken\n");
    let errors = compile_at("import type broken\n", &main).unwrap_err();
    assert_error(&errors, "expected 'end' to close 'declare class Dog'");

    project.definition(
        "dup",
        "declare class Dog\n    name: string\n    function name()\nend\n",
    );
    let errors = compile_at("import type dup\n", &main).unwrap_err();
    assert_error(&errors, "member 'name' is declared more than once in 'Dog'");
}

const PART_LUARD: &str = "declare class Part is\n    friend class Instance\n    public is\n        Anchored = false\n        CanCollide = true\n    end\n\n    private is\n        static function new()\n            \n        end\n    end\nend\n\ndeclare class Instance is\n    public is\n        static function new(name: string)\n            if name == \"Part\" then\n                return Part.new()\n            else\n                print(`unknown class: {name}`)\n                return nil\n            end\n        end\n    end\nend\n";

#[test]
fn luard_declare_class_with_bodies_friends_and_access_blocks() {
    let project = TempProject::new();
    project.definition("Part", PART_LUARD);
    let main = project.source("main.luar", "");
    let source = "import type Part\n\nfunction main()\n    if part := Instance.new(\"Part\") then\n        print(part.Anchored)\n        part.Anchored = true\n        print(part.Anchored)\n    end\nend\nmain()\n";
    let output = compile_at(source, &main).expect("the declaration sample should compile");
    assert!(output.contains("Instance.new(\"Part\")"), "{output}");
    assert!(!output.contains("class") && !output.contains("setmetatable"), "no runtime code for declarations: {output}");
}

#[test]
fn luard_declared_private_members_are_not_accessible_from_outside() {
    let project = TempProject::new();
    project.definition("Part", PART_LUARD);
    let main = project.source("main.luar", "");
    let errors = compile_at("import type Part\nlocal p = Part.new()\n", &main).unwrap_err();
    assert_error(&errors, "cannot access private method 'new' of class 'Part'");
}

#[test]
fn luard_declare_class_body_syntax_errors_are_reported() {
    let project = TempProject::new();
    project.definition("Broken", "declare class Broken is\n    public is\n        static function new(\n    end\nend\n");
    let main = project.source("main.luar", "");
    let errors = compile_at("import type Broken\n", &main).unwrap_err();
    assert_error(&errors, "Broken.luard");
}

#[test]
fn luard_declare_class_accepts_bodyless_methods_and_types_the_result() {
    let project = TempProject::new();
    project.definition(
        "Part",
        "declare class Part is\n    friend class Instance\n    public is\n        Anchored = false\n        CanCollide = true\n    end\n\n    private is\n        static function new(): Part\n    end\nend\n\ndeclare class Instance is\n    public is\n        static function new(name: string): Part?\n    end\nend",
    );
    let main = project.source("main.luar", "");
    let source = "import type Part\n\nfunction main()\n    if part := Instance.new(\"Part\") then\n        print(part.Anchored)\n        part.Anchored = true\n        print(part.Anchored)\n    end\nend\nmain()\n";
    let output = compile_at(source, &main).expect("bodyless declaration should compile");
    assert!(output.contains("Instance.new(\"Part\")"), "{output}");

    let errors = compile_at(
        "import type Part\nlocal p: number = Instance.new(\"Part\")\n",
        &main,
    )
    .unwrap_err();
    assert_error(&errors, "cannot assign Part? to 'p: number'");
}
