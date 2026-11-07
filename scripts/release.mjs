#!/usr/bin/env node
// Publie une nouvelle version de OneForAll sur GitHub.
//
//   npm run release              → version suivante (0.2.0 → 0.2.1)
//   npm run release -- minor     → 0.2.0 → 0.3.0
//   npm run release -- major     → 0.2.0 → 1.0.0
//   npm run release -- 1.4.2     → version précise
//   npm run release -- patch --notes "Correction du lecteur"
//
// Étapes : nouvelle version → compilation signée (clé TMDB intégrée) → latest.json →
// commit + tag + push → release GitHub publique avec l'installateur.
//
// Pré-requis (sur la machine qui publie, jamais dans le dépôt) :
//   .env.local   ONEFORALL_TMDB_KEY=...            (clé TMDB intégrée à l'app)
//                TAURI_SIGNING_PRIVATE_KEY_PATH=... (défaut : ~/.tauri/oneforall.key)
//                TAURI_SIGNING_PRIVATE_KEY_PASSWORD=...  (si la clé a un mot de passe)
//                GITHUB_TOKEN=...                  (facultatif : sinon le jeton de Git est utilisé)

import { execSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const REPO = "Mondherlol/OneForAll-Torrent-Client";
const run = (cmd, opts = {}) => execSync(cmd, { cwd: ROOT, stdio: "inherit", ...opts });
const out = (cmd) => execSync(cmd, { cwd: ROOT, encoding: "utf8" }).trim();
const step = (msg) => console.log(`\n\x1b[36m▶ ${msg}\x1b[0m`);
const fail = (msg) => {
  console.error(`\n\x1b[31m✖ ${msg}\x1b[0m`);
  process.exit(1);
};

// ---------- arguments ----------
const args = process.argv.slice(2);
const notesIdx = args.indexOf("--notes");
const notesArg = notesIdx >= 0 ? args[notesIdx + 1] : null;
const bump = args.find((a, i) => !a.startsWith("--") && (notesIdx < 0 || i !== notesIdx + 1)) ?? "patch";

// ---------- secrets locaux ----------
const envFile = path.join(ROOT, ".env.local");
if (fs.existsSync(envFile)) {
  for (const line of fs.readFileSync(envFile, "utf8").split(/\r?\n/)) {
    const m = line.match(/^\s*([A-Z0-9_]+)\s*=\s*(.*)\s*$/);
    if (m && !process.env[m[1]]) process.env[m[1]] = m[2].replace(/^["']|["']$/g, "");
  }
}
const keyPath = process.env.TAURI_SIGNING_PRIVATE_KEY_PATH || path.join(os.homedir(), ".tauri", "oneforall.key");
if (!fs.existsSync(keyPath)) fail(`Clé de signature introuvable : ${keyPath}\n  Crée-la avec : npx tauri signer generate -w "${keyPath}"`);
if (!process.env.ONEFORALL_TMDB_KEY) console.warn("⚠ ONEFORALL_TMDB_KEY absent de .env.local : l'app sera publiée sans clé TMDB intégrée.");

// ---------- version ----------
const pkgPath = path.join(ROOT, "package.json");
const confPath = path.join(ROOT, "src-tauri", "tauri.conf.json");
const cargoPath = path.join(ROOT, "src-tauri", "Cargo.toml");
const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));
const [maj, min, pat] = pkg.version.split(".").map(Number);
const version = /^\d+\.\d+\.\d+$/.test(bump)
  ? bump
  : bump === "major"
    ? `${maj + 1}.0.0`
    : bump === "minor"
      ? `${maj}.${min + 1}.0`
      : `${maj}.${min}.${pat + 1}`;
const tag = `v${version}`;

if (out("git status --porcelain").length && !args.includes("--allow-dirty")) {
  console.log("ℹ Des fichiers modifiés seront inclus dans le commit de la release.");
}
if (out(`git tag -l ${tag}`)) fail(`Le tag ${tag} existe déjà.`);

step(`Version ${pkg.version} → ${version}`);
pkg.version = version;
fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
const conf = JSON.parse(fs.readFileSync(confPath, "utf8"));
conf.version = version;
fs.writeFileSync(confPath, JSON.stringify(conf, null, 2) + "\n");
fs.writeFileSync(cargoPath, fs.readFileSync(cargoPath, "utf8").replace(/^version = ".*"$/m, `version = "${version}"`));

// ---------- compilation signée ----------
step("Compilation (signée, clé TMDB intégrée)…");
const env = {
  ...process.env,
  TAURI_SIGNING_PRIVATE_KEY: fs.readFileSync(keyPath, "utf8"),
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD: process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? "",
};
const build = spawnSync("npx", ["tauri", "build"], { cwd: ROOT, stdio: "inherit", env, shell: true });
if (build.status !== 0) fail("La compilation a échoué.");

// Dossier de sortie : target-dir de .cargo/config.toml s'il existe, sinon src-tauri/target
let targetDir = path.join(ROOT, "src-tauri", "target");
const cargoCfg = path.join(ROOT, "src-tauri", ".cargo", "config.toml");
if (fs.existsSync(cargoCfg)) {
  const m = fs.readFileSync(cargoCfg, "utf8").match(/target-dir\s*=\s*"([^"]+)"/);
  if (m) targetDir = path.resolve(path.join(ROOT, "src-tauri"), m[1]);
}
const nsisDir = path.join(targetDir, "release", "bundle", "nsis");
const setupName = `${conf.productName}_${version}_x64-setup.exe`;
const setupPath = path.join(nsisDir, setupName);
const sigPath = `${setupPath}.sig`;
if (!fs.existsSync(setupPath) || !fs.existsSync(sigPath)) fail(`Installateur ou signature introuvable dans ${nsisDir}`);

// ---------- notes ----------
const prevTag = (() => {
  try {
    return execSync("git describe --tags --abbrev=0 HEAD~1", { cwd: ROOT, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
  } catch {
    return null;
  }
})();
const notes =
  notesArg ??
  (() => {
    const log = out(`git log ${prevTag ? `${prevTag}..HEAD` : "-20"} --pretty=format:"- %s"`);
    return log || `Version ${version}`;
  })();

// ---------- latest.json (lu par les apps installées pour se mettre à jour) ----------
const downloadUrl = `https://github.com/${REPO}/releases/download/${tag}/${setupName}`;
const latest = {
  version,
  notes,
  pub_date: new Date().toISOString(),
  platforms: { "windows-x86_64": { signature: fs.readFileSync(sigPath, "utf8").trim(), url: downloadUrl } },
};
const latestPath = path.join(nsisDir, "latest.json");
fs.writeFileSync(latestPath, JSON.stringify(latest, null, 2));

// ---------- git ----------
step("Commit, tag et envoi sur GitHub…");
run("git add -A");
try {
  run(`git commit -m "Release ${tag}"`);
} catch {
  console.log("(rien à commiter)");
}
run(`git tag -a ${tag} -m "OneForAll ${version}"`);
run("git push --follow-tags origin HEAD");

// ---------- release GitHub ----------
step("Création de la release GitHub…");
function githubToken() {
  if (process.env.GITHUB_TOKEN) return process.env.GITHUB_TOKEN;
  // Jeton déjà enregistré par Git (Git Credential Manager)
  const r = spawnSync("git", ["credential", "fill"], { cwd: ROOT, input: "protocol=https\nhost=github.com\n\n", encoding: "utf8" });
  const pw = r.stdout?.match(/^password=(.+)$/m)?.[1];
  if (!pw) fail("Aucun jeton GitHub : définis GITHUB_TOKEN dans .env.local ou connecte-toi avec git push.");
  return pw;
}
const token = githubToken();
const api = async (url, init = {}) => {
  const res = await fetch(url, {
    ...init,
    headers: { Authorization: `Bearer ${token}`, Accept: "application/vnd.github+json", "User-Agent": "oneforall-release", ...(init.headers ?? {}) },
  });
  const body = await res.text();
  if (!res.ok) fail(`GitHub ${res.status} : ${body}`);
  return body ? JSON.parse(body) : null;
};
const release = await api(`https://api.github.com/repos/${REPO}/releases`, {
  method: "POST",
  body: JSON.stringify({ tag_name: tag, name: `OneForAll ${version}`, body: notes, draft: false, prerelease: false }),
});
for (const file of [setupPath, sigPath, latestPath]) {
  const name = path.basename(file);
  process.stdout.write(`  ↑ ${name}… `);
  await api(`https://uploads.github.com/repos/${REPO}/releases/${release.id}/assets?name=${encodeURIComponent(name)}`, {
    method: "POST",
    headers: { "Content-Type": "application/octet-stream" },
    body: fs.readFileSync(file),
  });
  console.log("ok");
}

console.log(`\n\x1b[32m✔ OneForAll ${version} publié : ${release.html_url}\x1b[0m`);
console.log("  Les apps installées verront la mise à jour au prochain démarrage.");
