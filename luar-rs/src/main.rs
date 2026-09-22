use luar_rs::{
    CompileOptions, Diagnostic, DiagnosticReport, Target, analyze_source_with_options,
    compile_source_with_options, dump_ir,
};
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

const VERSION: &str = concat!("luar ", env!("CARGO_PKG_VERSION"));

const HELP: &str = "luar - Luar compiler

Usage:
  luar compile [--target luau|lua54] <input.luar> [output]
  luar check [--target luau|lua54] <input.luar>
  luar check [--target luau|lua54] --stdin --source-path <path> [--diagnostic-format json]
  luar dump-ir [--target luau|lua54] <input.luar>
  luar help

The default target is luau. Output extensions do not select a target.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Command {
    Compile,
    Check,
    DumpIr,
}

struct Cli {
    command: Command,
    target: Target,
    stdin: bool,
    source_path: Option<PathBuf>,
    json: bool,
    positional: Vec<PathBuf>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(()) => ExitCode::FAILURE,
    }
}

fn run() -> Result<(), ()> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments.is_empty() || matches!(arguments[0].as_str(), "help" | "--help" | "-h") {
        println!("{HELP}");
        return Ok(());
    }
    if matches!(arguments[0].as_str(), "version" | "--version" | "-V") {
        println!("{VERSION}");
        return Ok(());
    }

    let cli = parse_cli(&arguments).map_err(|message| {
        eprintln!("luar: {message}");
        eprintln!("Run 'luar help' for usage.");
    })?;
    let (source, source_path) = read_input(&cli)?;
    let options = CompileOptions {
        target: cli.target,
        source_path: Some(source_path.clone()),
    };

    match cli.command {
        Command::Compile => {
            let output = compile_source_with_options(&source, &options)
                .map_err(|diagnostics| print_diagnostics(&diagnostics, cli.json))?;
            if let Some(output_path) = cli.positional.get(1) {
                fs::write(output_path, output).map_err(|error| {
                    eprintln!("luar: cannot write '{}': {error}", output_path.display());
                })?;
                println!("wrote {}", output_path.display());
            } else {
                io::stdout().write_all(output.as_bytes()).map_err(|error| {
                    eprintln!("luar: cannot write to stdout: {error}");
                })?;
            }
        }
        Command::Check => match analyze_source_with_options(&source, &options) {
            Ok(analysis) => {
                if cli.json {
                    print_json(&analysis.diagnostics)?;
                } else {
                    print_diagnostics(&analysis.diagnostics, false);
                    println!("{}: ok", source_path.display());
                }
            }
            Err(diagnostics) => {
                print_diagnostics(&diagnostics, cli.json);
                return Err(());
            }
        },
        Command::DumpIr => {
            let output = dump_ir(&source, &options)
                .map_err(|diagnostics| print_diagnostics(&diagnostics, cli.json))?;
            print!("{output}");
        }
    }
    Ok(())
}

fn parse_cli(arguments: &[String]) -> Result<Cli, String> {
    let command = match arguments[0].as_str() {
        "compile" => Command::Compile,
        "check" => Command::Check,
        "dump-ir" => Command::DumpIr,
        command => return Err(format!("unknown command '{command}'")),
    };
    let mut target = Target::Luau;
    let mut stdin = false;
    let mut source_path = None;
    let mut json = false;
    let mut positional = Vec::new();
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--target" => {
                index += 1;
                let value = arguments.get(index).ok_or("--target requires a value")?;
                target = value.parse()?;
            }
            "--stdin" => stdin = true,
            "--source-path" => {
                index += 1;
                let value = arguments
                    .get(index)
                    .ok_or("--source-path requires a value")?;
                source_path = Some(PathBuf::from(value));
            }
            "--diagnostic-format" => {
                index += 1;
                let value = arguments
                    .get(index)
                    .ok_or("--diagnostic-format requires a value")?;
                if value != "json" {
                    return Err(format!(
                        "unknown diagnostic format '{value}'; expected 'json'"
                    ));
                }
                json = true;
            }
            option if option.starts_with('-') => {
                return Err(format!("unknown option '{option}'"));
            }
            value => positional.push(PathBuf::from(value)),
        }
        index += 1;
    }

    if stdin {
        if command != Command::Check {
            return Err("--stdin is currently supported only by 'check'".to_string());
        }
        if source_path.is_none() {
            return Err("--stdin requires --source-path for module/include resolution".to_string());
        }
        if !positional.is_empty() {
            return Err("an input file cannot be combined with --stdin".to_string());
        }
    } else if positional.is_empty() {
        return Err("missing input file".to_string());
    }
    let maximum = if command == Command::Compile { 2 } else { 1 };
    if positional.len() > maximum {
        return Err("too many positional arguments".to_string());
    }
    if source_path.is_some() && !stdin {
        return Err("--source-path is only valid with --stdin".to_string());
    }

    Ok(Cli {
        command,
        target,
        stdin,
        source_path,
        json,
        positional,
    })
}

fn read_input(cli: &Cli) -> Result<(String, PathBuf), ()> {
    if cli.stdin {
        let mut source = String::new();
        io::stdin().read_to_string(&mut source).map_err(|error| {
            eprintln!("luar: cannot read stdin: {error}");
        })?;
        return Ok((
            source,
            cli.source_path.clone().expect("validated source path"),
        ));
    }
    let input = cli.positional.first().expect("validated input").clone();
    let source = fs::read_to_string(&input).map_err(|error| {
        eprintln!("luar: cannot read '{}': {error}", input.display());
    })?;
    Ok((source, input))
}

fn print_diagnostics(diagnostics: &[Diagnostic], json: bool) {
    if json {
        let _ = print_json(diagnostics);
        return;
    }
    for diagnostic in diagnostics {
        eprintln!(
            "{}:{}:{}: error: {}",
            diagnostic.file, diagnostic.line, diagnostic.column, diagnostic.message
        );
    }
}

fn print_json(diagnostics: &[Diagnostic]) -> Result<(), ()> {
    let report = DiagnosticReport {
        diagnostics: diagnostics.to_vec(),
    };
    serde_json::to_writer(io::stdout(), &report).map_err(|error| {
        eprintln!("luar: cannot write JSON diagnostics: {error}");
    })?;
    println!();
    Ok(())
}
