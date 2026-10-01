use luar_rs::CompileOptions;
use luar_rs::navigation::{Location, SemanticToken, definition, semantic_tokens};
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
        let path = std::env::temp_dir().join(format!("luar-nav-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn write(&self, name: &str, source: &str) {
        fs::write(self.path.join(name), source).unwrap();
    }

    fn options(&self, name: &str) -> CompileOptions {
        CompileOptions {
            source_path: Some(self.path.join(name)),
            ..CompileOptions::default()
        }
    }

    fn tokens(&self, source: &str) -> Vec<SemanticToken> {
        semantic_tokens(source, &self.options("main.luar"))
    }

    /// `|` をカーソル位置として、`main.luar` の定義を求める。
    fn definition(&self, source_with_cursor: &str) -> Vec<Location> {
        self.definition_in("main.luar", source_with_cursor)
    }

    fn definition_in(&self, name: &str, source_with_cursor: &str) -> Vec<Location> {
        let index = source_with_cursor.find('|').expect("cursor marker");
        let source = source_with_cursor.replacen('|', "", 1);
        let offset = source[..index].encode_utf16().count();
        definition(&source, offset, &self.options(name))
    }

    fn file(&self, name: &str) -> String {
        self.path.join(name).display().to_string()
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn token_at<'a>(tokens: &'a [SemanticToken], line: usize, column: usize) -> &'a SemanticToken {
    tokens
        .iter()
        .find(|token| token.line == line && token.column == column)
        .unwrap_or_else(|| panic!("no token at {line}:{column} in {tokens:#?}"))
}

fn summary(token: &SemanticToken) -> (&str, Vec<&str>) {
    (
        token.kind.as_str(),
        token.modifiers.iter().map(String::as_str).collect(),
    )
}

#[test]
fn semantic_tokens_distinguish_variables_constants_functions_and_members() {
    let project = TempProject::new();
    let tokens = project.tokens(
        "local count = 1\nconst NAME = \"x\"\nfunction go(arg)\n    print(count, NAME, arg)\nend\nlocal t = { key = 1 }\nprint(t.key)\nt.run()\n",
    );
    assert_eq!(summary(token_at(&tokens, 0, 6)), ("variable", vec!["declaration"]));
    assert_eq!(summary(token_at(&tokens, 1, 0)), ("keyword", vec![]));
    assert_eq!(
        summary(token_at(&tokens, 1, 6)),
        ("variable", vec!["declaration", "readonly"])
    );
    assert_eq!(summary(token_at(&tokens, 2, 9)), ("function", vec!["declaration"]));
    assert_eq!(summary(token_at(&tokens, 2, 12)), ("parameter", vec!["declaration"]));
    assert_eq!(summary(token_at(&tokens, 3, 4)).0, "function");
    assert_eq!(summary(token_at(&tokens, 3, 10)).0, "variable");
    assert_eq!(summary(token_at(&tokens, 3, 17)), ("variable", vec!["readonly"]));
    assert_eq!(summary(token_at(&tokens, 3, 23)).0, "parameter");
    assert_eq!(summary(token_at(&tokens, 5, 12)), ("property", vec!["declaration"]));
    assert_eq!(summary(token_at(&tokens, 6, 8)).0, "property");
    assert_eq!(summary(token_at(&tokens, 7, 2)).0, "method");
}

#[test]
fn semantic_tokens_cover_classes_and_ignore_included_files() {
    let project = TempProject::new();
    project.write(
        "clsdef.luar",
        "local module = {}\nclass Dog is\n    public is\n        name = \"x\"\n    end\nend\nmodule = { dog = Dog.new() }\nreturn module\n",
    );
    let tokens = project.tokens(
        "local clsdef = !include(\"./clsdef.luar\")\nclass Cat is\n    public is\n        function meow()\n        end\n    end\nend\nprint(clsdef.dog.name)\n",
    );
    assert_eq!(
        summary(token_at(&tokens, 0, 6)),
        ("namespace", vec!["declaration"])
    );
    assert_eq!(summary(token_at(&tokens, 1, 6)), ("class", vec!["declaration"]));
    assert_eq!(summary(token_at(&tokens, 3, 17)), ("method", vec!["declaration"]));
    assert_eq!(summary(token_at(&tokens, 7, 6)).0, "namespace");
    assert_eq!(summary(token_at(&tokens, 7, 13)).0, "property");
    assert!(
        tokens.iter().all(|token| token.line <= 7),
        "tokens from the included file must not leak: {tokens:#?}"
    );
    let sorted = tokens
        .windows(2)
        .all(|pair| (pair[0].line, pair[0].column) < (pair[1].line, pair[1].column));
    assert!(sorted, "tokens must be sorted without duplicates");
}

#[test]
fn definition_in_the_same_file_follows_scopes() {
    let project = TempProject::new();
    let locations = project.definition("local x = 1\ndo\n    local x = 2\n    print(x|)\nend\n");
    assert_eq!(locations.len(), 1);
    assert_eq!((locations[0].line, locations[0].column), (2, 10));
    assert_eq!(locations[0].file, project.file("main.luar"));

    let function = project.definition("function go() end\ngo|()\n");
    assert_eq!((function[0].line, function[0].column), (0, 9));

    let parameter = project.definition("function go(arg)\n    return ar|g\nend\n");
    assert_eq!((parameter[0].line, parameter[0].column), (0, 12));
}

#[test]
fn definition_of_class_members_uses_the_receiver_type() {
    let project = TempProject::new();
    let source = "class Dog is\n    public is\n        name = \"x\"\n        function bark()\n        end\n    end\nend\nclass Cat is\n    public is\n        name = \"y\"\n    end\nend\nlocal d = Dog.new()\nprint(d.name)\nd.bark()\n";
    let locations = project.definition(&source.replace("d.name", "d.na|me"));
    assert_eq!(locations.len(), 1, "{locations:#?}");
    assert_eq!((locations[0].line, locations[0].column), (2, 8), "Dog.name, not Cat.name");

    let method = project.definition(&source.replace("d.bark()", "d.ba|rk()"));
    assert_eq!((method[0].line, method[0].column), (3, 17));
}

#[test]
fn definition_follows_includes_and_shape_fields() {
    let project = TempProject::new();
    project.write(
        "clsdef.luar",
        "local module = {}\nclass Dog is\n    public is\n        name = \"Pochi\"\n    end\nend\n\nmodule = {\n    dog = Dog.new()\n}\n\nreturn module\n",
    );
    let main = "local clsdef = !include(\"./clsdef.luar\")\nprint(clsdef.dog.name)\n";

    let name = project.definition(&main.replace("dog.name", "dog.na|me"));
    assert_eq!(name.len(), 1, "{name:#?}");
    assert_eq!(name[0].file, project.file("clsdef.luar"));
    assert_eq!((name[0].line, name[0].column), (3, 8));

    let dog = project.definition(&main.replace("clsdef.dog", "clsdef.do|g"));
    assert_eq!(dog.len(), 1, "{dog:#?}");
    assert_eq!(dog[0].file, project.file("clsdef.luar"));
    assert_eq!((dog[0].line, dog[0].column), (8, 4));
}

#[test]
fn definition_of_the_include_path_and_binding_opens_the_file() {
    let project = TempProject::new();
    project.write("clsdef.luar", "local module = {}\nreturn module\n");
    let path = project.definition("local clsdef = !include(\"./cls|def.luar\")\n");
    assert_eq!(path.len(), 1);
    assert!(path[0].file.ends_with("clsdef.luar"), "{path:#?}");
    assert_eq!((path[0].line, path[0].column), (0, 0));

    let binding = project.definition("local cls|def = !include(\"./clsdef.luar\")\n");
    assert!(binding[0].file.ends_with("clsdef.luar"), "{binding:#?}");
}

#[test]
fn definition_reaches_into_luard_files() {
    let project = TempProject::new();
    project.write(
        "Part.luard",
        "declare class Part is\n    friend class Instance\n    public is\n        Anchored = false\n    end\n    private is\n        static function new(): Part\n    end\nend\n\ndeclare class Instance is\n    public is\n        static function new(name: string): Part?\n    end\nend\ndeclare version: string\n",
    );
    let main = "import type Part\nif part := Instance.new(\"Part\") then\n    print(part.Anchored)\nend\nprint(version)\n";

    let anchored = project.definition(&main.replace("part.Anchored", "part.Anch|ored"));
    assert_eq!(anchored.len(), 1, "{anchored:#?}");
    assert!(anchored[0].file.ends_with("Part.luard"));
    assert_eq!((anchored[0].line, anchored[0].column), (3, 8));

    let class = project.definition(&main.replace("Instance.new", "Insta|nce.new"));
    assert!(class[0].file.ends_with("Part.luard"), "{class:#?}");
    assert_eq!(class[0].line, 10);

    let method = project.definition(&main.replace("Instance.new", "Instance.ne|w"));
    assert_eq!(method[0].line, 12, "{method:#?}");

    let global = project.definition(&main.replace("print(version)", "print(vers|ion)"));
    assert_eq!(global[0].line, 15, "{global:#?}");

    let module = project.definition(&main.replace("import type Part", "import type Pa|rt"));
    assert!(module[0].file.ends_with("Part.luard"));
    assert_eq!(module[0].line, 0);
}

#[test]
fn definition_inside_a_luard_resolves_class_names() {
    let project = TempProject::new();
    let locations = project.definition_in(
        "Part.luard",
        "declare class Part is\n    public is\n        Anchored = false\n    end\nend\ndeclare dog: Pa|rt\n",
    );
    assert_eq!(locations.len(), 1, "{locations:#?}");
    assert_eq!((locations[0].line, locations[0].column), (0, 14));
}

#[test]
fn unanalyzable_input_returns_nothing_instead_of_failing() {
    let project = TempProject::new();
    assert!(project.tokens("local s = \"unfinished").is_empty());
    assert!(project.definition("print(|x)\n").len() <= 1);
}

#[test]
fn luard_declared_class_names_are_highlighted_as_classes() {
    let project = TempProject::new();
    project.write(
        "Part.luard",
        "declare class Instance is\n    public is\n        static function new(name: string): Part?\n    end\nend\n",
    );
    let tokens = project.tokens("import type Part\nlocal p = Instance.new(\"Part\")\n");
    assert_eq!(summary(token_at(&tokens, 1, 10)).0, "class");
}
