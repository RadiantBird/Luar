use luar_rs::{CompileOptions, Severity, analyze_source_with_options, compile_source};
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
        let path = std::env::temp_dir().join(format!("luar-rs-defs-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn definition(&self, source: &str) {
        fs::write(self.path.join("m.luard"), source).unwrap();
    }

    fn options(&self) -> CompileOptions {
        CompileOptions {
            source_path: Some(self.path.join("main.luar")),
            ..CompileOptions::default()
        }
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// (エラー文, 警告文) を返す。
fn analyze(definition: &str, source: &str) -> (Vec<String>, Vec<String>) {
    let project = TempProject::new();
    project.definition(definition);
    let source = format!("import type m\n{source}");
    match analyze_source_with_options(&source, &project.options()) {
        Ok(analysis) => split(analysis.diagnostics),
        Err(diagnostics) => split(diagnostics),
    }
}

fn split(diagnostics: Vec<luar_rs::Diagnostic>) -> (Vec<String>, Vec<String>) {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for diagnostic in diagnostics {
        match diagnostic.severity {
            Severity::Error => errors.push(diagnostic.message),
            Severity::Warning => warnings.push(diagnostic.message),
        }
    }
    (errors, warnings)
}

fn compile(source: &str) -> Result<String, Vec<String>> {
    compile_source(source, None)
}

fn assert_has(messages: &[String], expected: &str) {
    assert!(
        messages.iter().any(|message| message.contains(expected)),
        "expected a message containing {expected:?}, got {messages:#?}"
    );
}

// ─── メソッドの self ─────────────────────────────────────────────────────────

const VECTOR: &str = r#"
declare class Vector3 is
    public is
        x: number
        static function new(x: number, y: number, z: number): Vector3
        function set(self, x: number, y: number, z: number): ()
        function length(self): number
        function scale(self: Vector3, k: number): Vector3
    end
end
"#;

#[test]
fn self_as_the_first_parameter_is_accepted_in_declarations() {
    let (errors, warnings) = analyze(
        VECTOR,
        "local v = Vector3.new(1, 2, 3)\nv.set(4, 5, 6)\nlocal n: number = v.length()\nlocal w = v.scale(2)\n",
    );
    assert_eq!(errors, Vec::<String>::new());
    assert_eq!(warnings, Vec::<String>::new());
}

#[test]
fn self_as_the_first_parameter_is_accepted_in_the_short_declaration_form() {
    let (errors, _) = analyze(
        "declare class Counter\n    value: number\n    function add(self, amount: number): number\nend\ndeclare c: Counter\n",
        "local n: number = m.c.add(1)\n",
    );
    assert_eq!(errors, Vec::<String>::new());
}

#[test]
fn self_in_a_source_class_is_not_a_second_parameter() {
    let output = compile(
        "class Counter is\n    public is\n        value = 0\n        function add(self, amount: number)\n            self.value = self.value + amount\n        end\n    end\nend\nlocal c = Counter.new()\nc.add(2)\n",
    )
    .expect("self may be written explicitly");
    assert!(!output.contains("self, self"), "{output}");
    assert!(output.contains("function Counter.add(self, amount)"), "{output}");
}

#[test]
fn a_static_method_cannot_take_self() {
    let errors = compile(
        "class A is\n    public is\n        static function make(self)\n            return 1\n        end\n    end\nend\n",
    )
    .unwrap_err();
    assert_has(&errors, "a static method has no 'self'");
}

// ─── 壊れた .luard ──────────────────────────────────────────────────────────

const BROKEN: &str = r#"
declare class Vector3 is
    public is
        function normalize(v Vector3): number
    end
end

declare answer: number

declare class Instance is
    public is
        Name: string
        static function new(classname: string): Instance?
    end
end
"#;

#[test]
fn a_broken_declaration_does_not_hide_the_others() {
    let (errors, _) = analyze(
        BROKEN,
        "local n: string = m.answer\nlocal i = Instance.new(\"Part\")\nlocal name: number = i.Name\n",
    );
    assert_has(&errors, "m.luard:4");
    // 壊れていない宣言は使えるので、その型検査は働く。
    assert_has(&errors, "cannot assign number to 'n: string'");
}

#[test]
fn unknown_global_warnings_are_not_reported_when_a_definition_is_broken() {
    let (errors, warnings) = analyze(BROKEN, "local v = Vector3.new(1, 2, 3)\nlocal i = Instance.new(\"Part\")\n");
    assert_has(&errors, "m.luard:4");
    assert_eq!(warnings, Vec::<String>::new(), "no cascade of unknown globals");
}

#[test]
fn unknown_globals_are_still_reported_when_every_definition_loads() {
    let (errors, warnings) = analyze("declare answer: number\n", "print(undefined_name)\n");
    assert_eq!(errors, Vec::<String>::new());
    assert_has(&warnings, "unknown global 'undefined_name'");
}

#[test]
fn definition_errors_are_reported_on_the_import_line() {
    let project = TempProject::new();
    project.definition("declare broken
");
    let source = "-- comment
import type m
print(1)
";
    let diagnostics = match analyze_source_with_options(source, &project.options()) {
        Ok(analysis) => analysis.diagnostics,
        Err(diagnostics) => diagnostics,
    };
    let error = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.severity == Severity::Error)
        .expect("a definition error");
    assert_eq!(error.line, 2);
    assert!(error.message.contains("m.luard"), "{}", error.message);
}

#[test]
fn each_broken_declaration_reports_one_error() {
    let (errors, _) = analyze(
        "declare a: number\ndeclare b number\ndeclare c: string\ndeclare d:\ndeclare e: boolean\n",
        "local x: number = m.a\n",
    );
    let definition_errors: Vec<_> = errors.iter().filter(|error| error.contains("m.luard")).collect();
    assert_eq!(definition_errors.len(), 2, "{errors:#?}");
}

#[test]
fn template_headers_are_kept_with_the_declaration_that_follows_a_broken_one() {
    let (errors, _) = analyze(
        "declare class A is\n    public is\n        function f(x y)\n    end\nend\n\ntemplate <T>\ndeclare function pick(a: T, b: T): T\n",
        "local n: number = m.pick(1, 2)\n",
    );
    assert_has(&errors, "m.luard:3");
    assert!(
        !errors.iter().any(|error| error.contains("unknown type 'T'")),
        "the template header must stay attached to `pick`: {errors:#?}"
    );
}

// ─── インスタンスメソッドは `:` で呼ぶ ──────────────────────────────────────

fn compile_with_module(definition: &str, source: &str) -> String {
    let project = TempProject::new();
    project.definition(definition);
    let source = format!("import type m\n{source}");
    compile_source(&source, project.options().source_path.as_deref())
        .unwrap_or_else(|errors| panic!("{errors:#?}"))
}

#[test]
fn instance_methods_of_declared_classes_are_called_with_a_colon() {
    let output = compile_with_module(
        VECTOR,
        "local v = Vector3.new(1, 2, 3)\nv.set(4, 5, 6)\nlocal n = v.length()\nlocal w = v.scale(2)\n",
    );
    assert!(output.contains("v:set(4, 5, 6)"), "{output}");
    assert!(output.contains("local n = v:length()"), "{output}");
    assert!(output.contains("local w = v:scale(2)"), "{output}");
}

#[test]
fn static_methods_keep_the_dot() {
    let output = compile_with_module(VECTOR, "local v = Vector3.new(1, 2, 3)
");
    assert!(output.contains("Vector3.new(1, 2, 3)"), "{output}");
}

#[test]
fn instance_methods_are_found_through_parameters_fields_and_optionals() {
    let definition = r#"
declare class Part is
    public is
        Position: Part
        function destroy(self): ()
        static function new(name: string): Part?
    end
end
"#;
    let output = compile_with_module(
        definition,
        "local p = Part.new(\"a\")\np.destroy()\nfunction clean(part: Part)\n    part.destroy()\n    part.Position.destroy()\nend\n",
    );
    assert!(output.contains("p:destroy()"), "{output}");
    assert!(output.contains("part:destroy()"), "{output}");
    assert!(output.contains("part.Position:destroy()"), "{output}");
}

#[test]
fn instance_method_calls_inside_expressions_are_rewritten() {
    let output = compile_with_module(
        VECTOR,
        "local v = Vector3.new(1, 2, 3)\nlocal t = { len = v.length() }\nprint(`{v.length()}`, math.floor(v.length()))\nif v.length() > 1 then\n    print(v.length())\nend\n",
    );
    assert!(output.contains("len = v:length()"), "{output}");
    assert!(output.contains("math.floor(v:length())"), "{output}");
    assert!(output.contains("if v:length() > 1 then"), "{output}");
    assert!(output.contains("print(v:length())"), "{output}");
    assert!(!output.contains("v.length()"), "{output}");
}

#[test]
fn source_class_instance_methods_use_a_colon_too() {
    let output = compile(
        "class Counter is\n    public is\n        value = 0\n        function add(amount: number)\n            self.value = self.value + amount\n        end\n    end\nend\nfunction bump(c: Counter)\n    c.add(1)\nend\nlocal c = Counter.new()\nc.add(2)\n",
    )
    .unwrap();
    assert!(output.contains("c:add(1)"), "{output}");
    assert!(output.contains("c:add(2)"), "{output}");
}

#[test]
fn plain_table_calls_are_left_alone() {
    let output = compile("local t = { run = function() end }\nt.run()\nstring.format(\"x\")\n").unwrap();
    assert!(output.contains("t.run()"), "{output}");
    assert!(output.contains("string.format(\"x\")"), "{output}");
}
