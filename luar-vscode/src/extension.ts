import * as path from "path";
import { ExtensionContext, workspace } from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

let client: LanguageClient;

export function activate(context: ExtensionContext) {
  const serverModule = context.asAbsolutePath(path.join("out", "server.js"));

  const serverOptions: ServerOptions = {
    run:   { module: serverModule, transport: TransportKind.ipc },
    debug: { module: serverModule, transport: TransportKind.ipc },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "luar" }],
    initializationOptions: compilerSettings(),
    synchronize: { configurationSection: "luar" },
  };

  client = new LanguageClient("luar", "Luar Language Server", serverOptions, clientOptions);
  client.start();
}

function compilerSettings() {
  const configuration = workspace.getConfiguration("luar");
  return {
    compilerPath: configuration.get<string>("compiler.path", "luar"),
    target: configuration.get<"luau" | "lua54">("target", "luau"),
  };
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}
