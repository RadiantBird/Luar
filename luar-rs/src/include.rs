use std::fs;
use std::path::{Path, PathBuf};

/// Expands `local name = !include("path")` declarations before lexing.
///
/// Includes deliberately operate on source text: an included module must end in
/// `return identifier`; that return is removed and the module body is pasted at
/// the declaration site.  This keeps the generated Luau independent of a
/// runtime `require` implementation.
pub fn expand_source(source: &str, source_path: Option<&Path>) -> Result<String, String> {
    let Some(source_path) = source_path else {
        if find_include_declaration(source).is_some() {
            return Err(
                "!include declarations require compile_source with a source file path".to_string(),
            );
        }
        return Ok(source.to_string());
    };
    if source_path.as_os_str().is_empty() && find_include_declaration(source).is_some() {
        return Err("!include declarations require a non-empty source file path".to_string());
    }

    let mut stack = Vec::new();
    expand_with_path(source, source_path, &mut stack)
}

fn expand_with_path(source: &str, source_path: &Path, stack: &mut Vec<PathBuf>) -> Result<String, String> {
    let directory = source_path.parent().unwrap_or_else(|| Path::new(""));
    let mut output: Vec<String> = Vec::new();

    for (index, line) in source.lines().enumerate() {
        let line_number = index + 1;
        let Some(declaration) = parse_include_declaration(line) else {
            output.push(line.to_string());
            continue;
        };

        let requested = declaration.path.strip_prefix('@').unwrap_or(&declaration.path);
        if Path::new(requested).is_absolute() {
            return Err(error_at(
                source_path,
                line_number,
                "!include paths must be relative to the including source file",
            ));
        }
        let include_path = directory.join(requested);
        if include_path.extension().and_then(|extension| extension.to_str()) != Some("luar") {
            return Err(error_at(
                source_path,
                line_number,
                "!include only accepts .luar source files",
            ));
        }

        let canonical_path = fs::canonicalize(&include_path).map_err(|error| {
            error_at(
                source_path,
                line_number,
                &format!("cannot read included source '{}': {error}", include_path.display()),
            )
        })?;
        if stack.contains(&canonical_path) {
            let cycle = stack
                .iter()
                .chain(std::iter::once(&canonical_path))
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(" -> ");
            return Err(error_at(
                source_path,
                line_number,
                &format!("!include cycle detected: {cycle}"),
            ));
        }

        let included_source = fs::read_to_string(&canonical_path).map_err(|error| {
            error_at(
                source_path,
                line_number,
                &format!("included source '{}' is not valid UTF-8: {error}", canonical_path.display()),
            )
        })?;
        let (module_body, returned_name) = remove_terminal_return(&included_source, &canonical_path)?;

        stack.push(canonical_path.clone());
        let expanded_body = expand_with_path(&module_body, &canonical_path, stack);
        stack.pop();
        let expanded_body = expanded_body?;

        output.push(expanded_body);
        if returned_name != declaration.name {
            output.push(format!(
                "{} {} = {}",
                declaration.binding, declaration.name, returned_name
            ));
        }
    }

    Ok(output.join("\n"))
}

struct IncludeDeclaration<'a> {
    binding: &'a str,
    name: &'a str,
    path: String,
}

fn find_include_declaration(source: &str) -> Option<()> {
    source.lines().find_map(|line| parse_include_declaration(line).map(|_| ()))
}

fn parse_include_declaration(line: &str) -> Option<IncludeDeclaration<'_>> {
    let trimmed = line.trim();
    let binding_end = trimmed.find(char::is_whitespace)?;
    let binding = &trimmed[..binding_end];
    if binding != "local" && binding != "const" {
        return None;
    }
    let remainder = trimmed[binding_end..].trim_start();
    let name_end = remainder
        .char_indices()
        .take_while(|(_, character)| character.is_ascii_alphanumeric() || *character == '_')
        .last()
        .map(|(index, character)| index + character.len_utf8())?;
    let name = &remainder[..name_end];
    if !name.chars().next()?.is_ascii_alphabetic() && !name.starts_with('_') {
        return None;
    }
    let remainder = remainder[name_end..].trim_start();
    let remainder = remainder.strip_prefix('=')?.trim_start();
    let remainder = remainder.strip_prefix("!include")?.trim_start();
    let remainder = remainder.strip_prefix('(')?.trim_start();
    let quote = remainder.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let remainder = &remainder[quote.len_utf8()..];
    let closing_quote = remainder.find(quote)?;
    let path = &remainder[..closing_quote];
    let remainder = remainder[closing_quote + 1..].trim_start();
    let remainder = remainder.strip_prefix(')')?.trim();
    if !remainder.is_empty() && !remainder.starts_with("--") {
        return None;
    }
    Some(IncludeDeclaration {
        binding,
        name,
        path: path.to_string(),
    })
}

fn remove_terminal_return(source: &str, source_path: &Path) -> Result<(String, String), String> {
    let mut lines = source.lines().collect::<Vec<_>>();
    let mut last_code = None;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if !trimmed.is_empty() && !trimmed.starts_with("--") {
            last_code = Some(index);
        }
    }
    let Some(return_index) = last_code else {
        return Err(format!(
            "{}:1: included source must end with standalone `return <identifier>`",
            source_path.display()
        ));
    };
    let return_line = lines[return_index];
    let Some(returned_name) = parse_terminal_return(return_line) else {
        return Err(format!(
            "{}:{}: included source must end with standalone `return <identifier>`",
            source_path.display(),
            return_index + 1
        ));
    };
    lines.remove(return_index);
    Ok((lines.join("\n"), returned_name.to_string()))
}

fn parse_terminal_return(line: &str) -> Option<&str> {
    let remainder = line.trim().strip_prefix("return")?;
    if !remainder.chars().next()?.is_whitespace() {
        return None;
    }
    let remainder = remainder.trim_start();
    let name_end = remainder
        .char_indices()
        .take_while(|(_, character)| character.is_ascii_alphanumeric() || *character == '_')
        .last()
        .map(|(index, character)| index + character.len_utf8())?;
    let name = &remainder[..name_end];
    if !name.chars().next()?.is_ascii_alphabetic() && !name.starts_with('_') {
        return None;
    }
    let tail = remainder[name_end..].trim();
    if tail.is_empty() || tail.starts_with("--") {
        Some(name)
    } else {
        None
    }
}

fn error_at(source_path: &Path, line: usize, message: &str) -> String {
    format!("{}:{line}: {message}", source_path.display())
}
