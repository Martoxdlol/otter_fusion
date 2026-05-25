import * as vscode from "vscode";
import { execFile, execFileSync } from "node:child_process";
import * as path from "node:path";

const LANGUAGE_ID = "otter-fusion";

let diagnostics: vscode.DiagnosticCollection;
let loginPath: string | undefined;

export function activate(context: vscode.ExtensionContext): void {
  diagnostics = vscode.languages.createDiagnosticCollection(LANGUAGE_ID);
  context.subscriptions.push(diagnostics);

  loginPath = loadLoginShellPath();

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

// Login zsh sources .zprofile; capture its PATH for execFile lookup.
function loadLoginShellPath(): string | undefined {
  try {
    const out = execFileSync("zsh", ["-lc", 'printf %s "$PATH"'], {
      timeout: 5_000,
      encoding: "utf8",
    });
    return out.trim() || process.env.PATH;
  } catch {
    return process.env.PATH;
  }
}

function resolveBinary(doc: vscode.TextDocument): string {
  const configured = vscode.workspace
    .getConfiguration("otterFusion")
    .get<string>("binaryPath", "otter_fusion");

  if (path.isAbsolute(configured)) return configured;

  const looksRelative =
    configured.includes("/") || configured.includes(path.sep);
  if (looksRelative) {
    const folder = vscode.workspace.getWorkspaceFolder(doc.uri);
    if (folder) return path.resolve(folder.uri.fsPath, configured);
  }

  return configured;
}

function validate(doc: vscode.TextDocument): void {
  if (doc.languageId !== LANGUAGE_ID) return;
  if (doc.uri.scheme !== "file") return;

  const bin = resolveBinary(doc);

  execFile(
    bin,
    ["validate", doc.fileName, "--short"],
    {
      timeout: 10_000,
      env: { ...process.env, PATH: loginPath ?? process.env.PATH },
    },
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
