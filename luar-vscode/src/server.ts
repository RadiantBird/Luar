import {
  createConnection,
  TextDocuments,
  ProposedFeatures,
  InitializeParams,
  TextDocumentSyncKind,
  InitializeResult,
  Diagnostic,
  DiagnosticSeverity,
  CompletionItem,
  CompletionItemKind,
  CompletionParams,
  DocumentSymbol,
  Hover,
  MarkupKind,
  Position,
  Range,
  SymbolKind as LspSymbolKind,
  DidChangeConfigurationParams,
} from "vscode-languageserver/node";
import { TextDocument } from "vscode-languageserver-textdocument";
import { fileURLToPath, pathToFileURL } from "node:url";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";

import {
  importsInDocument,
  indexDocument,
  loadModuleDefinition,
  normalizeIncludeMacros,
  type DocumentIndex,
  type LanguageSymbol,
} from "./language";

const connection = createConnection(ProposedFeatures.all);
const documents = new TextDocuments(TextDocument);
const indexes = new Map<string, DocumentIndex>();
const moduleDiagnostics = new Map<string, Diagnostic[]>();
const validationTimers = new Map<string, NodeJS.Timeout>();
const validationProcesses = new Map<string, ChildProcessWithoutNullStreams>();
const publishedCompilerUris = new Map<string, Set<string>>();
const compilerDiagnosticsByOwner = new Map<string, Map<string, Diagnostic[]>>();

interface CompilerSettings {
  compilerPath: string;
  target: "luau" | "lua54";
}

interface CompilerDiagnostic {
  file: string;
  line: number;
  column: number;
  endLine: number;
  endColumn: number;
  severity: "error" | "warning";
  message: string;
}

let settings: CompilerSettings = { compilerPath: "luar", target: "luau" };
let compilerMissingWasReported = false;

connection.onInitialize((params: InitializeParams): InitializeResult => {
  settings = parseSettings(params.initializationOptions);
  return {
    capabilities: {
      textDocumentSync: TextDocumentSyncKind.Incremental,
      completionProvider: { triggerCharacters: ["."] },
      hoverProvider: true,
      documentSymbolProvider: true,
    },
  };
});

documents.onDidChangeContent(({ document }) => { updateIndex(document); scheduleValidation(document); });
documents.onDidOpen(({ document }) => { updateIndex(document); scheduleValidation(document); });
documents.onDidClose(({ document }) => {
  indexes.delete(document.uri);
  moduleDiagnostics.delete(document.uri);
  cancelValidation(document.uri);
  clearCompilerDiagnostics(document.uri);
  connection.sendDiagnostics({ uri: document.uri, diagnostics: [] });
});

connection.onDidChangeConfiguration((params: DidChangeConfigurationParams) => {
  settings = parseSettings((params.settings as { luar?: unknown } | undefined)?.luar ?? params.settings);
  compilerMissingWasReported = false;
  for (const document of documents.all()) scheduleValidation(document);
});

connection.onCompletion((params: CompletionParams): CompletionItem[] => {
  const document = documents.get(params.textDocument.uri);
  if (!document) return [];
  const index = getIndex(document);
  const before = document.getText({ start: { line: params.position.line, character: 0 }, end: params.position });
  if (/\bimport\s+[A-Za-z_0-9]*$/.test(before)) {
    return [{ label: "type", kind: CompletionItemKind.Keyword, detail: "type-only module import" }];
  }
  const gotoMatch = before.match(/\bgoto\s+([A-Za-z_][A-Za-z0-9_]*)?$/);
  if (gotoMatch) {
    const labelPrefix = gotoMatch[1] ?? "";
    return index.symbols
      .filter((symbol) => symbol.kind === "label" && symbol.name.startsWith(labelPrefix))
      .map(completionFor);
  }
  const memberMatch = before.match(/([A-Za-z_][A-Za-z0-9_]*)\.([A-Za-z_][A-Za-z0-9_]*)?$/);
  if (memberMatch) {
    const memberPrefix = memberMatch[2] ?? "";
    const members = index.symbols.filter((symbol) =>
      symbol.parent === memberMatch[1] && symbol.name.startsWith(memberPrefix),
    );
    return members.map((symbol) => completionFor(symbol));
  }
  const prefix = /[A-Za-z_][A-Za-z0-9_]*$/.exec(before)?.[0] ?? "";
  return index.symbols.filter((symbol) => !symbol.parent && symbol.name.startsWith(prefix)).map(completionFor).concat(
    index.keywords
      .filter((keyword) => keyword.startsWith(prefix))
      .map((keyword) => ({ label: keyword, kind: CompletionItemKind.Keyword })),
  );
});

connection.onHover((params): Hover | null => {
  const document = documents.get(params.textDocument.uri);
  if (!document) return null;
  const word = wordAt(document, params.position);
  if (!word) return null;
  const index = getIndex(document);
  const lineText = document.getText({ start: { line: params.position.line, character: 0 }, end: { line: params.position.line + 1, character: 0 } }).split("\n")[0] ?? "";
  const receiver = lineText.slice(0, params.position.character).match(/([A-Za-z_][A-Za-z0-9_]*)\.[A-Za-z_][A-Za-z0-9_]*$/)?.[1];
  const symbol = index.symbols.find((candidate) => candidate.name === word && (!receiver || candidate.parent === receiver))
    ?? index.symbols.find((candidate) => candidate.name === word);
  if (!symbol) return null;
  return { contents: { kind: MarkupKind.Markdown, value: `\`luar\n${symbol.signature}\n\`` } };
});

connection.onDocumentSymbol((params): DocumentSymbol[] => {
  const document = documents.get(params.textDocument.uri);
  if (!document) return [];
  const index = getIndex(document);
  return index.symbols.filter((symbol) => !symbol.parent).map((symbol) => {
    // Module members come from another file, so their source ranges are not
    // valid ranges in this document. Keep the import itself in the outline.
    const children = symbol.kind === "module"
      ? []
      : index.symbols.filter((child) => child.parent === symbol.name).map(toDocumentSymbol);
    const result = toDocumentSymbol(symbol);
    if (children.length) result.children = children;
    return result;
  });
});

function updateIndex(document: TextDocument): void {
  const modules = resolveModules(document);
  indexes.set(document.uri, indexDocument(normalizeIncludeMacros(document.getText()), modules.definitions));
  moduleDiagnostics.set(document.uri, modules.diagnostics);
}
function getIndex(document: TextDocument): DocumentIndex {
  const current = indexes.get(document.uri);
  if (current) return current;
  const created = indexDocument(normalizeIncludeMacros(document.getText()));
  indexes.set(document.uri, created);
  return created;
}

function resolveModules(document: TextDocument): { definitions: NonNullable<ReturnType<typeof loadModuleDefinition>["definition"]>[]; diagnostics: Diagnostic[] } {
  const definitions: NonNullable<ReturnType<typeof loadModuleDefinition>["definition"]>[] = [];
  const diagnostics: Diagnostic[] = [];
  const imports = importsInDocument(document.getText());
  const seen = new Set<string>();
  let sourcePath: string;
  try {
    sourcePath = fileURLToPath(document.uri);
  } catch {
    return { definitions, diagnostics };
  }

  for (const imported of imports) {
    if (seen.has(imported.name)) {
      diagnostics.push({
        severity: DiagnosticSeverity.Error,
        range: importRange(imported.line, imported.col, imported.name),
        message: `module '${imported.name}' is imported more than once`,
        source: "luar",
      });
      continue;
    }
    seen.add(imported.name);
    const result = loadModuleDefinition(imported.name, sourcePath);
    if (result.definition) definitions.push(result.definition);
    if (result.errors.length === 0) continue;
    for (const error of result.errors) {
      diagnostics.push({
        severity: DiagnosticSeverity.Error,
        range: errorRange(error, imported.line, imported.col),
        message: error.message,
        source: "luar",
      });
    }
  }
  return { definitions, diagnostics };
}

function importRange(line: number, col: number, name: string): Range {
  return Range.create(line, col, line, col + name.length);
}

function errorRange(error: { line: number; col: number }, fallbackLine: number, fallbackCol: number): Range {
  const line = error.line >= 0 ? error.line : fallbackLine;
  const col = error.col >= 0 ? error.col : fallbackCol;
  return Range.create(line, col, line, col + 1);
}
function completionFor(symbol: LanguageSymbol): CompletionItem {
  const kinds: Record<LanguageSymbol["kind"], CompletionItemKind> = { class: CompletionItemKind.Class, method: CompletionItemKind.Method, field: CompletionItemKind.Field, function: CompletionItemKind.Function, variable: CompletionItemKind.Variable, module: CompletionItemKind.Module, label: CompletionItemKind.Reference };
  return { label: symbol.name, kind: kinds[symbol.kind], detail: symbol.signature, documentation: symbol.signature };
}
function toDocumentSymbol(symbol: LanguageSymbol): DocumentSymbol {
  const kind: Record<LanguageSymbol["kind"], LspSymbolKind> = { class: LspSymbolKind.Class, method: LspSymbolKind.Method, field: LspSymbolKind.Field, function: LspSymbolKind.Function, variable: LspSymbolKind.Variable, module: LspSymbolKind.Namespace, label: LspSymbolKind.Key };
  const range = Range.create(symbol.line, symbol.col, symbol.endLine, symbol.endCol);
  return DocumentSymbol.create(symbol.name, symbol.signature, kind[symbol.kind], range, range);
}
function wordAt(document: TextDocument, position: Position): string | null {
  const line = document.getText({ start: { line: position.line, character: 0 }, end: { line: position.line + 1, character: 0 } }).split("\n")[0] ?? "";
  const before = line.slice(0, position.character);
  const after = line.slice(position.character);
  const match = before.match(/[A-Za-z_][A-Za-z0-9_]*$/);
  const suffix = after.match(/^[A-Za-z0-9_]*/)?.[0] ?? "";
  return match ? `${match[0]}${suffix}` : null;
}

function scheduleValidation(document: TextDocument): void {
  cancelValidation(document.uri);
  if (document.uri.toLowerCase().endsWith(".luard")) {
    connection.sendDiagnostics({ uri: document.uri, diagnostics: moduleDiagnostics.get(document.uri) ?? [] });
    return;
  }
  const version = document.version;
  validationTimers.set(document.uri, setTimeout(() => {
    validationTimers.delete(document.uri);
    runCompilerValidation(document, version);
  }, 250));
}

function cancelValidation(uri: string): void {
  const timer = validationTimers.get(uri);
  if (timer) clearTimeout(timer);
  validationTimers.delete(uri);
  validationProcesses.get(uri)?.kill();
  validationProcesses.delete(uri);
}

function runCompilerValidation(document: TextDocument, version: number): void {
  let sourcePath: string;
  try {
    sourcePath = fileURLToPath(document.uri);
  } catch {
    return;
  }

  const child = spawn(settings.compilerPath, [
    "check", "--target", settings.target, "--stdin",
    "--source-path", sourcePath, "--diagnostic-format", "json",
  ], { windowsHide: true });
  validationProcesses.set(document.uri, child);

  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk: string) => { stdout += chunk; });
  child.stderr.on("data", (chunk: string) => { stderr += chunk; });
  child.on("error", (error: NodeJS.ErrnoException) => {
    validationProcesses.delete(document.uri);
    if (error.code === "ENOENT" && !compilerMissingWasReported) {
      compilerMissingWasReported = true;
      connection.window.showWarningMessage(
        `Luar compiler '${settings.compilerPath}' was not found. Configure luar.compiler.path to enable semantic diagnostics.`,
      );
    } else if (!compilerMissingWasReported) {
      compilerMissingWasReported = true;
      connection.window.showErrorMessage(`Luar compiler could not start: ${error.message}`);
    }
    clearCompilerDiagnostics(document.uri);
  });
  child.stdin.on("error", () => {
    // The process-level error/close handlers above own user-visible reporting.
  });
  child.on("close", () => {
    if (validationProcesses.get(document.uri) !== child) return;
    validationProcesses.delete(document.uri);
    const current = documents.get(document.uri);
    if (!current || current.version !== version) return;

    let diagnostics: CompilerDiagnostic[];
    try {
      const parsed = JSON.parse(stdout) as { diagnostics?: CompilerDiagnostic[] } | CompilerDiagnostic[];
      diagnostics = Array.isArray(parsed) ? parsed : (parsed.diagnostics ?? []);
    } catch {
      const message = stderr.trim() || stdout.trim() || "Luar compiler returned invalid diagnostic output";
      diagnostics = [{
        file: sourcePath,
        line: 1,
        column: 1,
        endLine: 1,
        endColumn: 2,
        severity: "error",
        message,
      }];
    }
    publishCompilerDiagnostics(document.uri, sourcePath, diagnostics);
  });
  child.stdin.end(document.getText());
}

function publishCompilerDiagnostics(ownerUri: string, sourcePath: string, diagnostics: CompilerDiagnostic[]): void {
  const previousUris = publishedCompilerUris.get(ownerUri) ?? new Set<string>();
  const byUri = new Map<string, Diagnostic[]>();
  for (const diagnostic of diagnostics) {
    const uri = pathToFileURL(diagnostic.file || sourcePath).toString();
    const list = byUri.get(uri) ?? [];
    list.push({
      severity: diagnostic.severity === "warning" ? DiagnosticSeverity.Warning : DiagnosticSeverity.Error,
      range: Range.create(
        Math.max(0, diagnostic.line - 1), Math.max(0, diagnostic.column - 1),
        Math.max(0, diagnostic.endLine - 1), Math.max(0, diagnostic.endColumn - 1),
      ),
      message: diagnostic.message,
      source: "luar",
    });
    byUri.set(uri, list);
  }

  const uris = new Set(byUri.keys());
  uris.add(ownerUri);
  compilerDiagnosticsByOwner.set(ownerUri, byUri);
  publishedCompilerUris.set(ownerUri, uris);
  for (const uri of new Set([...previousUris, ...uris])) publishUriDiagnostics(uri);
}

function clearCompilerDiagnostics(ownerUri: string): void {
  const affected = publishedCompilerUris.get(ownerUri) ?? new Set<string>();
  compilerDiagnosticsByOwner.delete(ownerUri);
  publishedCompilerUris.delete(ownerUri);
  for (const uri of affected) publishUriDiagnostics(uri);
}

function publishUriDiagnostics(uri: string): void {
  const diagnostics = [...(moduleDiagnostics.get(uri) ?? [])];
  for (const byUri of compilerDiagnosticsByOwner.values()) {
    diagnostics.push(...(byUri.get(uri) ?? []));
  }
  connection.sendDiagnostics({ uri, diagnostics });
}

function parseSettings(value: unknown): CompilerSettings {
  const candidate = value && typeof value === "object" ? value as Record<string, unknown> : {};
  const compilerPath = typeof candidate.compilerPath === "string"
    ? candidate.compilerPath
    : typeof candidate["compiler.path"] === "string"
      ? candidate["compiler.path"] as string
      : candidate.compiler && typeof candidate.compiler === "object" &&
          typeof (candidate.compiler as Record<string, unknown>).path === "string"
        ? (candidate.compiler as Record<string, unknown>).path as string
      : "luar";
  const target = candidate.target === "lua54" ? "lua54" : "luau";
  return { compilerPath, target };
}

documents.listen(connection);
connection.listen();
