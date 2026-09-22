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
} from "vscode-languageserver/node";
import { TextDocument } from "vscode-languageserver-textdocument";

import { Parser } from "../../luar/src/parser/parser";
import { Checker } from "../../luar/src/checker/checker";
import { indexDocument, type DocumentIndex, type LanguageSymbol } from "./language";

const connection = createConnection(ProposedFeatures.all);
const documents = new TextDocuments(TextDocument);
const indexes = new Map<string, DocumentIndex>();

connection.onInitialize((_params: InitializeParams): InitializeResult => ({
  capabilities: {
    textDocumentSync: TextDocumentSyncKind.Incremental,
    completionProvider: { triggerCharacters: ["."] },
    hoverProvider: true,
    documentSymbolProvider: true,
  },
}));

documents.onDidChangeContent(({ document }) => { updateIndex(document); validate(document); });
documents.onDidOpen(({ document }) => { updateIndex(document); validate(document); });
documents.onDidClose(({ document }) => {
  indexes.delete(document.uri);
  connection.sendDiagnostics({ uri: document.uri, diagnostics: [] });
});

connection.onCompletion((params: CompletionParams): CompletionItem[] => {
  const document = documents.get(params.textDocument.uri);
  if (!document) return [];
  const index = getIndex(document);
  const before = document.getText({ start: { line: params.position.line, character: 0 }, end: params.position });
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
  const symbol = index.symbols.find((candidate) => candidate.name === word);
  if (!symbol) return null;
  return { contents: { kind: MarkupKind.Markdown, value: `\`luar\n${symbol.signature}\n\`` } };
});

connection.onDocumentSymbol((params): DocumentSymbol[] => {
  const document = documents.get(params.textDocument.uri);
  if (!document) return [];
  const index = getIndex(document);
  return index.symbols.filter((symbol) => !symbol.parent).map((symbol) => {
    const children = index.symbols.filter((child) => child.parent === symbol.name).map(toDocumentSymbol);
    const result = toDocumentSymbol(symbol);
    if (children.length) result.children = children;
    return result;
  });
});

function updateIndex(document: TextDocument): void { indexes.set(document.uri, indexDocument(document.getText())); }
function getIndex(document: TextDocument): DocumentIndex {
  const current = indexes.get(document.uri);
  if (current) return current;
  const created = indexDocument(document.getText());
  indexes.set(document.uri, created);
  return created;
}
function completionFor(symbol: LanguageSymbol): CompletionItem {
  const kinds: Record<LanguageSymbol["kind"], CompletionItemKind> = { class: CompletionItemKind.Class, method: CompletionItemKind.Method, field: CompletionItemKind.Field, function: CompletionItemKind.Function, variable: CompletionItemKind.Variable, module: CompletionItemKind.Module };
  return { label: symbol.name, kind: kinds[symbol.kind], detail: symbol.signature, documentation: symbol.signature };
}
function toDocumentSymbol(symbol: LanguageSymbol): DocumentSymbol {
  const kind: Record<LanguageSymbol["kind"], LspSymbolKind> = { class: LspSymbolKind.Class, method: LspSymbolKind.Method, field: LspSymbolKind.Field, function: LspSymbolKind.Function, variable: LspSymbolKind.Variable, module: LspSymbolKind.Namespace };
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

function validate(document: TextDocument): void {
  const src = document.getText();
  const diagnostics: Diagnostic[] = [];

  try {
    const prog = new Parser(src).parse();
    const errors = new Checker().check(prog);

    for (const e of errors) {
      diagnostics.push({
        severity: DiagnosticSeverity.Error,
        range: {
          start: { line: e.line - 1, character: e.col - 1 },
          end:   { line: e.line - 1, character: e.col - 1 + 1 },
        },
        message: e.message,
        source: "luar",
      });
    }
  } catch (e: unknown) {
    if (e instanceof Error) {
      const located = e as Error & { line?: number; col?: number };
      const line = typeof located.line === "number" ? located.line - 1 : 0;
      const col  = typeof located.col  === "number" ? located.col  - 1 : 0;
      diagnostics.push({
        severity: DiagnosticSeverity.Error,
        range: {
          start: { line, character: col },
          end:   { line, character: col + 1 },
        },
        message: e.message,
        source: "luar",
      });
    }
  }

  connection.sendDiagnostics({ uri: document.uri, diagnostics });
}

documents.listen(connection);
connection.listen();
