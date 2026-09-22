use std::io::Write;
use std::process::{Command, Stdio};

fn luar() -> Command {
    Command::new(env!("CARGO_BIN_EXE_luar"))
}

#[test]
fn check_stdin_emits_the_editor_json_diagnostic_schema() {
    let mut child = luar()
        .args([
            "check",
            "--target",
            "luau",
            "--stdin",
            "--source-path",
            "C:\\project\\main.luar",
            "--diagnostic-format",
            "json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"goto missing")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());

    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let diagnostic = &report["diagnostics"][0];
    assert_eq!(diagnostic["file"], "C:\\project\\main.luar");
    assert_eq!(diagnostic["line"], 1);
    assert_eq!(diagnostic["column"], 1);
    assert_eq!(diagnostic["endLine"], 1);
    assert_eq!(diagnostic["endColumn"], 1);
    assert_eq!(diagnostic["severity"], "error");
    assert!(
        diagnostic["message"]
            .as_str()
            .unwrap()
            .contains("undefined label")
    );
}

#[test]
fn dump_ir_accepts_an_explicit_target() {
    let source = std::env::temp_dir().join(format!("luar-cli-ir-{}.luar", std::process::id()));
    std::fs::write(&source, "local value = 1\nreturn value").unwrap();
    let output = luar()
        .args(["dump-ir", "--target", "lua54"])
        .arg(&source)
        .output()
        .unwrap();
    std::fs::remove_file(&source).unwrap();

    assert!(output.status.success());
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(output.contains("function <chunk>"));
    assert!(output.contains("Return"));
}
