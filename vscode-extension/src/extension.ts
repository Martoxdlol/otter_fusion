import * as vscode from "vscode";
import { execFile } from "node:child_process";

const LANGUAGE_ID = "otter-fusion";

let diagnostics: vscode.DiagnosticCollection;

export function activate(context: vscode.ExtensionContext): void {
  diagnostics = vscode.languages.createDiagnosticCollection(LANGUAGE_ID);
  context.subscriptions.push(diagnostics);

  context.subscriptions.push(
    vscode.workspace.onDidSaveTextDocument(validate),
    vscode.workspace.onDidOpenTextDocument(validate),
    vscode.workspace.onDidCloseTextDocument((doc) => diagnostics.delete(doc.uri)),
  );

  vscode.workspace.textDocuments.forEach(validate);
}

export function deactivate(): void {
  diagnostics?.dispose();
}

function validate(doc: vscode.TextDocument): void {
  if (doc.languageId !== LANGUAGE_ID) return;
  if (doc.uri.scheme !== "file") return;

  const bin = vscode.workspace
    .getConfiguration("otterFusion")
    .get<string>("binaryPath", "otter_fusion");

  execFile(
    bin,
    ["validate", doc.fileName],
    { timeout: 10_000 },
    (err, stdout, stderr) => {
      if (err && (err as NodeJS.ErrnoException).code === "ENOENT") {
        vscode.window.showErrorMessage(
          `Otter Fusion: binary "${bin}" not found. Set otterFusion.binaryPath in settings.`,
        );
        diagnostics.set(doc.uri, []);
        return;
      }
      const output = (stderr || "") + (stdout || "");
      diagnostics.set(doc.uri, parseDiagnostics(output, doc));
    },
  );
}

const DIAGNOSTIC_LINE = /^(.+?):(\d+):(\d+):\s*(?:(error|warning|note|info):\s*)?(.*)$/i;

function parseDiagnostics(
  output: string,
  doc: vscode.TextDocument,
): vscode.Diagnostic[] {
  const out: vscode.Diagnostic[] = [];
  for (const raw of output.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;

    const m = DIAGNOSTIC_LINE.exec(line);
    if (!m) continue;

    const lineNum = Math.max(0, parseInt(m[2], 10) - 1);
    const colNum = Math.max(0, parseInt(m[3], 10) - 1);
    const severity = mapSeverity(m[4]);
    const message = m[5];

    const wordRange = doc.getWordRangeAtPosition(
      new vscode.Position(lineNum, colNum),
    );
    const range =
      wordRange ??
      new vscode.Range(lineNum, colNum, lineNum, colNum + 1);

    const diag = new vscode.Diagnostic(range, message, severity);
    diag.source = "otter_fusion";
    out.push(diag);
  }
  return out;
}

function mapSeverity(label: string | undefined): vscode.DiagnosticSeverity {
  switch ((label || "").toLowerCase()) {
    case "warning":
      return vscode.DiagnosticSeverity.Warning;
    case "note":
    case "info":
      return vscode.DiagnosticSeverity.Information;
    default:
      return vscode.DiagnosticSeverity.Error;
  }
}
