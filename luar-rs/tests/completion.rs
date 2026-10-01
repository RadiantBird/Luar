use luar_rs::completion::CompletionItem;
use luar_rs::{CompileOptions, complete_source_with_options};
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
        let path = std::env::temp_dir().join(format!("luar-complete-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn write(&self, name: &str, source: &str) {
        fs::write(self.path.join(name), source).unwrap();
    }

    /// `|` をカーソル位置として補完を求める。
    fn complete(&self, source_with_cursor: &str) -> Vec<CompletionItem> {
        let index = source_with_cursor.find('|').expect("cursor marker");
        let source = source_with_cursor.replacen('|', "", 1);
        let offset = source[..index].encode_utf16().count();
        let options = CompileOptions {
            source_path: Some(self.path.join("main.luar")),
            ..CompileOptions::default()
        };
        complete_source_with_options(&source, offset, &options).expect("completion should work")
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn labels(items: &[CompletionItem]) -> Vec<&str> {
    items.iter().map(|item| item.label.as_str()).collect()
}

fn item<'a>(items: &'a [CompletionItem], label: &str) -> &'a CompletionItem {
    items
        .iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("missing {label:?} in {:?}", labels(items)))
}

const CLSDEF: &str = "local module = {}\nclass Dog is\n    private is\n        secret = 1\n    end\n    public is\n        name:string = \"Pochi\"\n        function bark(times: number): string\n            return \"wan\"\n        end\n        static function create(): Dog\n            return Dog.new()\n        end\n    end\nend\n\nmodule = {\n    dog = Dog.new()\n}\n\nreturn module\n";

#[test]
fn include_member_chain_offers_fields_with_types() {
    let project = TempProject::new();
    project.write("clsdef.luar", CLSDEF);
    let items = project.complete("local clsdef = !include(\"./clsdef.luar\")\nprint(clsdef.dog.|)");
    assert_eq!(labels(&items), vec!["bark", "name"]);
    let name = item(&items, "name");
    assert_eq!((name.kind.as_str(), name.type_text.as_str()), ("field", "string"));
    assert_eq!(name.detail, "name: string");
    let bark = item(&items, "bark");
    assert_eq!(bark.kind, "method");
    assert_eq!(bark.detail, "function bark(times: number): string");
    assert_eq!(bark.type_text, "string");
}

#[test]
fn module_table_members_show_their_shapes() {
    let project = TempProject::new();
    project.write("clsdef.luar", CLSDEF);
    let items = project.complete("local clsdef = !include(\"./clsdef.luar\")\nclsdef.|");
    assert_eq!(labels(&items), vec!["dog"]);
    assert_eq!(item(&items, "dog").type_text, "Dog");
}

#[test]
fn partially_typed_input_without_closing_parenthesis_still_completes() {
    let project = TempProject::new();
    project.write("clsdef.luar", CLSDEF);
    let items = project.complete("local clsdef = !include(\"./clsdef.luar\")\nprint(clsdef.dog.na|");
    assert!(labels(&items).contains(&"name"));
}

#[test]
fn private_members_are_hidden_and_class_objects_offer_statics() {
    let project = TempProject::new();
    let instance = project.complete(
        "class Dog is\n    private is\n        secret = 1\n    end\n    public is\n        name = \"x\"\n    end\nend\nlocal d = Dog.new()\nd.|",
    );
    assert_eq!(labels(&instance), vec!["name"]);

    let class_object = project.complete(
        "class Dog is\n    public is\n        name = \"x\"\n        static function create(): Dog\n            return Dog.new()\n        end\n    end\nend\nDog.|",
    );
    assert_eq!(labels(&class_object), vec!["create", "new"]);
    assert_eq!(item(&class_object, "new").detail, "static function new(): Dog");
}

#[test]
fn inherited_members_are_offered_from_ancestors() {
    let project = TempProject::new();
    let items = project.complete(
        "class Animal is\n    public is\n        legs = 4\n    end\nend\nclass Dog is Animal\n    public is\n        name = \"x\"\n    end\nend\nlocal d = Dog.new()\nd.|",
    );
    assert_eq!(labels(&items), vec!["name", "legs"]);
    assert_eq!(item(&items, "legs").type_text, "number");
}

#[test]
fn scope_completion_lists_kinds_and_types() {
    let project = TempProject::new();
    let items = project.complete(
        "local count: number = 1\nconst NAME = \"x\"\nfunction go() end\nclass Dog is\nend\nlocal d = Dog.new()\n|",
    );
    let count = item(&items, "count");
    assert_eq!((count.kind.as_str(), count.detail.as_str()), ("variable", "local count: number"));
    let name = item(&items, "NAME");
    assert_eq!((name.kind.as_str(), name.detail.as_str()), ("constant", "const NAME: string"));
    assert_eq!(item(&items, "go").kind, "function");
    assert_eq!(item(&items, "Dog").kind, "class");
    assert_eq!(item(&items, "d").type_text, "Dog");
}

#[test]
fn luard_declared_classes_complete_without_source() {
    let project = TempProject::new();
    project.write(
        "ext.luard",
        "declare class Dog\n    name: string\n    function bark(times: number): string\nend\ndeclare dog: Dog\ndeclare version: string\n",
    );
    let members = project.complete("import type ext\nlocal ext = require(\"./ext\")\next.|");
    assert_eq!(labels(&members), vec!["dog", "version"]);
    assert_eq!(item(&members, "version").type_text, "string");

    let dog = project.complete("import type ext\nlocal ext = require(\"./ext\")\next.dog.|");
    assert_eq!(labels(&dog), vec!["bark", "name"]);
    assert_eq!(item(&dog, "bark").detail, "function bark(times: number): string");
}

#[test]
fn unanalyzable_source_reports_an_error_instead_of_guessing() {
    let project = TempProject::new();
    let options = CompileOptions {
        source_path: Some(project.path.join("main.luar")),
        ..CompileOptions::default()
    };
    assert!(complete_source_with_options("local = = \n", 0, &options).is_err());
}
