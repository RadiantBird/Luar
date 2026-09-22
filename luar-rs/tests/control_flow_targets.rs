use luar_rs::{
    CompileOptions, DiagnosticReport, Target, check_source_with_options, compile_source,
    compile_source_with_options, dump_ir, luar_compile_target,
};
use std::ffi::{CStr, CString};
use std::fs;
use std::ptr;

fn options(target: Target) -> CompileOptions {
    CompileOptions {
        target,
        source_path: None,
    }
}

const NESTED_EXIT: &str = r#"
function main()
    for i = 1, 10 do
        for j = 1, 10 do
            if i == 1 and j == 3 then
                goto exit
            end
            print(i, j)
        end
    end
    print("end for loop")
    ::exit::
    print("exit")
end
main()
"#;

#[test]
fn nested_loop_exit_uses_native_goto_for_lua54() {
    let output = compile_source_with_options(NESTED_EXIT, &options(Target::Lua54)).unwrap();
    assert!(output.contains("goto exit"));
    assert!(output.contains("::exit::"));
}

#[test]
fn nested_loop_exit_uses_dispatcher_for_luau() {
    let output = compile_source_with_options(NESTED_EXIT, &options(Target::Luau)).unwrap();
    assert!(output.contains("__luar_pc_"));
    assert!(!output.contains("goto exit"));
    assert!(!output.contains("::exit::"));
    assert!(output.contains("print(\"exit\")"));
}

#[test]
fn flow_validation_reports_unsafe_or_invalid_control_flow() {
    let undefined = check_source_with_options("goto missing", &options(Target::Lua54)).unwrap_err();
    assert!(
        undefined
            .iter()
            .any(|error| error.message.contains("undefined label 'missing'"))
    );

    let into_local = check_source_with_options(
        "goto after\nlocal value = 1\n::after::\nprint(value)",
        &options(Target::Lua54),
    )
    .unwrap_err();
    assert!(
        into_local
            .iter()
            .any(|error| error.message.contains("jumps into the scope of a local"))
    );

    let outside_loop =
        check_source_with_options("break\ncontinue", &options(Target::Luau)).unwrap_err();
    assert!(outside_loop.iter().any(|error| {
        error
            .message
            .contains("break is only allowed inside a loop")
    }));
    assert!(outside_loop.iter().any(|error| {
        error
            .message
            .contains("continue is only allowed inside a loop")
    }));

    let nested_label =
        check_source_with_options("if true then\n::inside::\nend", &options(Target::Lua54))
            .unwrap_err();
    assert!(
        nested_label
            .iter()
            .any(|error| error.message.contains("nested labels are not supported"))
    );
}

#[test]
fn interpolation_is_target_specific_and_default_remains_luau() {
    let source = "local name = \"Luar\"\nlocal text = `hello {name}`";
    let default_output = compile_source(source, None).unwrap();
    let luau = compile_source_with_options(source, &options(Target::Luau)).unwrap();
    let lua54 = compile_source_with_options(source, &options(Target::Lua54)).unwrap();

    assert_eq!(default_output, luau);
    assert!(luau.contains("`hello {name}`"));
    assert!(lua54.contains("\"hello \" .. tostring(name)"));
}

#[test]
fn dump_ir_and_json_diagnostic_schema_are_available() {
    let ir = dump_ir("local value = 1\nreturn value", &options(Target::Luau)).unwrap();
    assert!(ir.contains("function <chunk>"));
    assert!(ir.contains("Return"));

    let diagnostics =
        check_source_with_options("goto nowhere", &options(Target::Luau)).unwrap_err();
    let json = serde_json::to_string(&DiagnosticReport { diagnostics }).unwrap();
    let report: serde_json::Value = serde_json::from_str(&json).unwrap();
    let diagnostic = &report["diagnostics"][0];
    assert!(diagnostic["file"].is_string());
    assert!(diagnostic["line"].as_u64().unwrap() >= 1);
    assert!(diagnostic["column"].as_u64().unwrap() >= 1);
    assert_eq!(diagnostic["severity"], "error");
}

#[test]
fn explicit_target_c_abi_keeps_old_default_api_separate() {
    let source = CString::new("const value = 1").unwrap();
    let target = CString::new("lua54").unwrap();
    let mut output = vec![0_i8; 256];
    let result = luar_compile_target(
        source.as_ptr(),
        target.as_ptr(),
        output.as_mut_ptr(),
        output.len(),
    );
    assert_eq!(result, 0);
    let output = unsafe { CStr::from_ptr(output.as_ptr()) }.to_str().unwrap();
    assert!(output.contains("local value <const> = 1"));

    let invalid_target = CString::new("lua51").unwrap();
    let result = luar_compile_target(source.as_ptr(), invalid_target.as_ptr(), ptr::null_mut(), 0);
    assert_eq!(result, -1);
}

#[test]
fn lua54_include_is_validated_and_preserved_without_luau_approximation() {
    let directory =
        std::env::temp_dir().join(format!("luar-rs-lua54-include-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let main = directory.join("main.luar");
    let legacy = directory.join("legacy.lua");
    fs::write(
        &legacy,
        "local mod <const> = {}\nfunction mod.run() print('legacy') end\nreturn mod\n",
    )
    .unwrap();
    let source = "local mod = !include(\"./legacy.lua\")\nmod.run()";

    let lua54 = compile_source_with_options(
        source,
        &CompileOptions {
            target: Target::Lua54,
            source_path: Some(main.clone()),
        },
    )
    .unwrap();
    assert!(lua54.contains("local mod <const> = {}"));
    assert!(!lua54.contains("return mod"));

    let luau = compile_source_with_options(
        source,
        &CompileOptions {
            target: Target::Luau,
            source_path: Some(main),
        },
    )
    .unwrap_err();
    assert!(luau.iter().any(|error| {
        error
            .message
            .contains("cannot be represented by the Luau target")
    }));

    fs::remove_file(legacy).unwrap();
    fs::remove_dir(directory).unwrap();
}
