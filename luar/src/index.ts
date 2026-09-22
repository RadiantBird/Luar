#!/usr/bin/env node
import { readFileSync, writeFileSync } from "node:fs";
import { extname } from "node:path";
import { Parser } from "./parser/parser.js";
import { Checker } from "./checker/checker.js";
import { Codegen } from "./codegen/codegen.js";

const args = process.argv.slice(2);
const cmd = args[0];

function printHelp() {
  console.log(`\
luar - Luar to Luau transpiler v0.1.0

Usage:
  luar compile <input.luar> [output.luau]   Compile Luar source to Luau
  luar check   <input.luar>                 Type-check without emitting output
  luar help                                 Show this help

Examples:
  luar compile hello.luar               # prints Luau to stdout
  luar compile hello.luar hello.luau    # writes hello.luau
  luar check   hello.luar               # exits 0 on success, 1 on error`);
}

if (!cmd || cmd === "help" || cmd === "--help" || cmd === "-h") {
  printHelp();
  process.exit(0);
}

if (cmd !== "compile" && cmd !== "check") {
  console.error(`luar: unknown command '${cmd}'`);
  console.error("Run 'luar help' for usage.");
  process.exit(1);
}

const inputPath = args[1];
if (!inputPath) {
  console.error(`luar ${cmd}: missing input file`);
  console.error("Run 'luar help' for usage.");
  process.exit(1);
}

let src: string;
try {
  src = readFileSync(inputPath, "utf-8");
} catch {
  console.error(`luar: cannot read '${inputPath}'`);
  process.exit(1);
}

// Parse
let prog;
try {
  prog = new Parser(src).parse();
} catch (e: unknown) {
  console.error(`luar: parse error — ${(e as Error).message}`);
  process.exit(1);
}

// Check
const errors = new Checker().check(prog);
if (errors.length > 0) {
  for (const e of errors) {
    console.error(`${inputPath}:${e.line}:${e.col}: error: ${e.message}`);
  }
  process.exit(1);
}

if (cmd === "check") {
  console.log(`${inputPath}: ok`);
  process.exit(0);
}

// Codegen (compile only)
const output = new Codegen().generate(prog);

const outputPath = args[2] ?? deriveOutputPath(inputPath);

if (args[2]) {
  writeFileSync(outputPath, output);
  console.log(`wrote ${outputPath}`);
} else {
  process.stdout.write(output);
}

function deriveOutputPath(input: string): string {
  const ext = extname(input);
  const base = ext ? input.slice(0, -ext.length) : input;
  return base + ".luau";
}
