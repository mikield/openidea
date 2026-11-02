/**
 * Exports the React frontend's keymaps and built-in themes as data for the GPUI
 * client: `client/crates/plugin-presets/data/{keymaps,themes}/*.json`.
 *
 *   node client/scripts/export-presets.ts          # write
 *   node client/scripts/export-presets.ts --check  # fail when the files are stale (CI)
 *
 * Needs Node >= 23 (runs TypeScript natively) and no packages: the keymaps are
 * imported straight from `src/platform/keymaps/*.ts`, and themes are resolved
 * from `src/styles/tokens.css` here, per built-in theme of `src/platform/themes.ts`:
 * the CSS cascade for its root attributes (as `src/workbench/theme.ts` sets
 * them), `var()` substituted and `color-mix()` evaluated.
 */
import { registerHooks } from "node:module";
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

// The frontend imports siblings without an extension (`./intellij-macos`).
registerHooks({
  resolve(specifier, context, next) {
    try {
      return next(specifier, context);
    } catch (e) {
      if (specifier.startsWith(".") && !specifier.endsWith(".ts")) return next(`${specifier}.ts`, context);
      throw e;
    }
  },
});

const repo = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const out = join(repo, "client", "crates", "plugin-presets", "data");
const check = process.argv.includes("--check");
const load = (path: string) => import(pathToFileURL(join(repo, path)).href);

/** The last upstream commit that touched the sources, recorded as the port revision. */
function revision(paths: string[]): string {
  try {
    return execFileSync("git", ["log", "-1", "--format=%h", "--", ...paths], { cwd: repo }).toString().trim();
  } catch {
    return "unknown";
  }
}

// ── keymaps ─────────────────────────────────────────────────────────────────

const KEYMAPS: Array<[id: string, module: string, name: string, title: string]> = [
  ["intellij-windows", "intellij-windows", "intellijWindows", "Windows"],
  ["intellij-macos", "intellij-macos", "intellijMacos", "macOS"],
  ["vscode-windows", "vscode-windows", "vscodeWindows", "VS Code (Windows)"],
  ["vscode-macos", "vscode-macos", "vscodeMacos", "VS Code (macOS)"],
  ["eclipse-windows", "eclipse-windows", "eclipseWindows", "Eclipse"],
  ["eclipse-macos", "eclipse-macos", "eclipseMacos", "Eclipse (macOS)"],
];

async function keymaps(): Promise<Record<string, unknown>> {
  const from = revision(["src/platform/keymaps"]);
  const files: Record<string, unknown> = {};
  for (const [id, module, name, title] of KEYMAPS) {
    const bindings: Record<string, string[]> = (await load(`src/platform/keymaps/${module}.ts`))[name];
    const sorted = Object.fromEntries(Object.entries(bindings).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));
    files[`keymaps/${id}.json`] = { id, title, from, bindings: sorted };
  }
  return files;
}

// ── themes ──────────────────────────────────────────────────────────────────

type Attrs = Record<string, string>;

/** Root attributes per built-in theme id (`applyThemeAttributes` in src/workbench/theme.ts). */
function themeAttrs(id: string, kind: string): Attrs {
  const attrs: Attrs = { "data-theme": kind };
  if (id === "dark-hc") attrs["data-contrast"] = "high";
  if (!["light", "dark", "dark-hc"].includes(id)) attrs["data-color-theme"] = id;
  return attrs;
}

interface Rule {
  selectors: string[];
  decls: Array<[string, string]>;
}

function parseRules(css: string): Rule[] {
  const src = css.replace(/\/\*[\s\S]*?\*\//g, "");
  const rules: Rule[] = [];
  let i = 0;
  while (i < src.length) {
    const open = src.indexOf("{", i);
    if (open < 0) break;
    const close = src.indexOf("}", open);
    const selectors = src.slice(i, open).split(",").map((s) => s.trim()).filter(Boolean);
    const decls: Array<[string, string]> = [];
    for (const decl of src.slice(open + 1, close).split(";")) {
      const colon = decl.indexOf(":");
      if (colon < 0) continue;
      const name = decl.slice(0, colon).trim();
      if (name.startsWith("--")) decls.push([name.slice(2), decl.slice(colon + 1).trim()]);
    }
    rules.push({ selectors, decls });
    i = close + 1;
  }
  return rules;
}

/** Specificity of `selector` when it matches a root with `attrs`, else -1. */
function specificity(selector: string, attrs: Attrs): number {
  if (selector === "html") return 1;
  if (!selector.startsWith(":root")) return -1;
  const rest = selector.slice(":root".length);
  const parts = [...rest.matchAll(/\[([a-z-]+)(?:="([^"]*)")?\]/g)];
  if (parts.map((m) => m[0]).join("") !== rest) return -1;
  for (const [, name, value] of parts) {
    if (!(name in attrs) || (value !== undefined && attrs[name] !== value)) return -1;
  }
  return 10 * (1 + parts.length);
}

function cascade(rules: Rule[], attrs: Attrs): Map<string, string> {
  const winners = new Map<string, { spec: number; order: number; value: string }>();
  let order = 0;
  for (const rule of rules) {
    const spec = Math.max(...rule.selectors.map((s) => specificity(s, attrs)));
    for (const [name, value] of rule.decls) {
      order++;
      if (spec < 0) continue;
      const prev = winners.get(name);
      if (!prev || spec > prev.spec || (spec === prev.spec && order > prev.order)) winners.set(name, { spec, order, value });
    }
  }
  return new Map([...winners].map(([name, w]) => [name, w.value]));
}

/** Index just past the `)` matching the `(` at `open`. */
function matching(s: string, open: number): number {
  let depth = 0;
  for (let i = open; i < s.length; i++) {
    if (s[i] === "(") depth++;
    else if (s[i] === ")" && --depth === 0) return i + 1;
  }
  return s.length;
}

/** Splits on commas outside parentheses. */
function args(s: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < s.length; i++) {
    if (s[i] === "(") depth++;
    else if (s[i] === ")") depth--;
    else if (s[i] === "," && depth === 0) {
      out.push(s.slice(start, i).trim());
      start = i + 1;
    }
  }
  out.push(s.slice(start).trim());
  return out;
}

function substitute(value: string, vars: Map<string, string>, seen: Set<string>): string {
  let s = value;
  for (let at = s.indexOf("var("); at >= 0; at = s.indexOf("var(")) {
    const end = matching(s, at + 3);
    const [ref, fallback] = args(s.slice(at + 4, end - 1));
    const name = ref.replace(/^--/, "");
    let resolved = fallback ?? "";
    if (vars.has(name) && !seen.has(name)) {
      resolved = substitute(vars.get(name)!, vars, new Set([...seen, name]));
    } else if (fallback !== undefined) {
      resolved = substitute(fallback, vars, seen);
    }
    s = s.slice(0, at) + resolved + s.slice(end);
  }
  return s;
}

type Rgba = [number, number, number, number];

function parseColor(raw: string): Rgba | null {
  const s = raw.trim().toLowerCase();
  if (s === "transparent") return [0, 0, 0, 0];
  if (s === "white") return [255, 255, 255, 1];
  if (s === "black") return [0, 0, 0, 1];
  const hex = /^#([0-9a-f]{3,8})$/.exec(s)?.[1];
  if (hex && [3, 4, 6, 8].includes(hex.length)) {
    const full = hex.length <= 4 ? [...hex].map((c) => c + c).join("") : hex;
    const n = (i: number) => parseInt(full.slice(i, i + 2), 16);
    return [n(0), n(2), n(4), full.length === 8 ? n(6) / 255 : 1];
  }
  const hsl = /^hsla?\((.*)\)$/.exec(s)?.[1];
  if (hsl) {
    const parts = hsl.split(/[\s,/]+/).filter(Boolean);
    if (parts.length < 3) return null;
    const h = (((parseFloat(parts[0]) % 360) + 360) % 360) / 360;
    const [sat, light] = [parseFloat(parts[1]) / 100, parseFloat(parts[2]) / 100];
    const alpha = parts[3] ? (parts[3].endsWith("%") ? parseFloat(parts[3]) / 100 : parseFloat(parts[3])) : 1;
    const q = light < 0.5 ? light * (1 + sat) : light + sat - light * sat;
    const p = 2 * light - q;
    const channel = (t: number) => {
      const u = t < 0 ? t + 1 : t > 1 ? t - 1 : t;
      const v = u < 1 / 6 ? p + (q - p) * 6 * u : u < 1 / 2 ? q : u < 2 / 3 ? p + (q - p) * (2 / 3 - u) * 6 : p;
      return v * 255;
    };
    return [channel(h + 1 / 3), channel(h), channel(h - 1 / 3), alpha];
  }
  const fn = /^rgba?\((.*)\)$/.exec(s)?.[1];
  if (fn) {
    const parts = fn.split(/[\s,/]+/).filter(Boolean);
    const num = (p: string, scale: number) => (p.endsWith("%") ? (parseFloat(p) / 100) * scale : parseFloat(p));
    if (parts.length < 3) return null;
    return [num(parts[0], 255), num(parts[1], 255), num(parts[2], 255), parts[3] ? num(parts[3], 1) : 1];
  }
  return null;
}

function toHex([r, g, b, a]: Rgba): string {
  const h = (v: number) => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, "0");
  return `#${h(r)}${h(g)}${h(b)}${a < 1 ? h(a * 255) : ""}`;
}

/** `color-mix(in srgb, A p%, B q%)` with premultiplied alpha, as CSS Color 5 defines it. */
function colorMix(inner: string): string | null {
  const [space, ...colors] = args(inner);
  if (space.replace(/\s+/g, " ") !== "in srgb" || colors.length !== 2) return null;
  const parsed = colors.map((c) => {
    const m = /^(.*?)\s+([\d.]+)%$/.exec(c);
    return { color: parseColor(evaluate(m ? m[1] : c)), pct: m ? parseFloat(m[2]) : undefined };
  });
  if (parsed.some((p) => !p.color)) return null;
  let [p1, p2] = parsed.map((p) => p.pct);
  if (p1 === undefined && p2 === undefined) [p1, p2] = [50, 50];
  else if (p1 === undefined) p1 = 100 - p2!;
  else if (p2 === undefined) p2 = 100 - p1;
  const sum = p1 + p2!;
  const [w1, w2] = [p1 / sum, p2! / sum];
  const [c1, c2] = [parsed[0].color!, parsed[1].color!];
  const alpha = c1[3] * w1 + c2[3] * w2;
  const channel = (i: number) => (alpha === 0 ? 0 : (c1[i] * c1[3] * w1 + c2[i] * c2[3] * w2) / alpha);
  return toHex([channel(0), channel(1), channel(2), alpha * Math.min(1, sum / 100)]);
}

/** Evaluates `color-mix()` innermost first and normalizes a lone colour to hex. */
function evaluate(value: string): string {
  let s = value.trim();
  for (let at = s.lastIndexOf("color-mix("); at >= 0; at = s.lastIndexOf("color-mix(")) {
    const end = matching(s, at + "color-mix".length);
    const mixed = colorMix(s.slice(at + "color-mix(".length, end - 1));
    if (mixed === null) break;
    s = s.slice(0, at) + mixed + s.slice(end);
  }
  const color = parseColor(s);
  return color ? toHex(color) : s;
}

async function themes(): Promise<Record<string, unknown>> {
  const { BUILTIN_THEMES } = await load("src/platform/themes.ts");
  const css = readFileSync(join(repo, "src/styles/tokens.css"), "utf8");
  const rules = parseRules(css);
  const from = revision(["src/styles/tokens.css", "src/platform/themes.ts"]);
  const files: Record<string, unknown> = {};
  for (const { id, label, kind } of BUILTIN_THEMES as Array<{ id: string; label: string; kind: string }>) {
    const vars = cascade(rules, themeAttrs(id, kind));
    const tokens: Record<string, string> = {};
    for (const name of [...vars.keys()].sort()) {
      tokens[name] = evaluate(substitute(vars.get(name)!, vars, new Set([name])));
    }
    files[`themes/${id}.json`] = { id, name: label, kind, from, tokens };
  }
  return files;
}

// ── write / check ───────────────────────────────────────────────────────────

const files = { ...(await keymaps()), ...(await themes()) };
const stale: string[] = [];
for (const [path, data] of Object.entries(files)) {
  const file = join(out, path);
  const text = `${JSON.stringify(data, null, 2)}\n`;
  let current = "";
  try {
    current = readFileSync(file, "utf8").replace(/\r\n/g, "\n");
  } catch {}
  if (current === text) continue;
  if (check) {
    // A shallow clone (CI) cannot name the revision: compare the data without it.
    const withoutFrom = (json: string) => {
      try {
        const { from: _, ...rest } = JSON.parse(json);
        return JSON.stringify(rest);
      } catch {
        return json;
      }
    };
    if (withoutFrom(current) !== withoutFrom(text)) stale.push(path);
  } else {
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, text);
    console.log(`wrote ${path}`);
  }
}
if (stale.length) {
  console.error(`stale, run \`node client/scripts/export-presets.ts\`: ${stale.join(", ")}`);
  process.exit(1);
}
