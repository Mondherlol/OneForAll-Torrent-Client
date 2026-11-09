#!/usr/bin/env node
// Récupère un jeton IGDB (Twitch, valable ~60 jours) et l'écrit dans .env.local.
//
//   npm run igdb-token            → nouveau jeton
//   npm run igdb-token -- --auto  → seulement s'il manque ou expire dans moins de 14 jours
//
// Lit ONEFORALL_IGDB_CLIENT_ID et ONEFORALL_IGDB_SECRET dans .env.local (jamais commité).
// « npm run release » l'appelle tout seul avec --auto.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const envFile = path.join(ROOT, ".env.local");

export async function refreshIgdbToken({ auto = false } = {}) {
  let text = fs.existsSync(envFile) ? fs.readFileSync(envFile, "utf8") : "";
  const get = (k) => text.match(new RegExp(`^\s*${k}\s*=\s*(.*?)\s*$`, "m"))?.[1]?.replace(/^["']|["']$/g, "");
  const id = get("ONEFORALL_IGDB_CLIENT_ID");
  const secret = get("ONEFORALL_IGDB_SECRET");
  if (!id || !secret) {
    console.warn("⚠ ONEFORALL_IGDB_CLIENT_ID / ONEFORALL_IGDB_SECRET absents de .env.local : pas de jeton IGDB.");
    return null;
  }
  const expires = Date.parse(get("ONEFORALL_IGDB_TOKEN_EXPIRES") ?? "");
  if (auto && get("ONEFORALL_IGDB_TOKEN") && expires - Date.now() > 14 * 86400e3) {
    console.log(`ℹ Jeton IGDB valable jusqu'au ${new Date(expires).toLocaleDateString("fr-FR")}.`);
    return get("ONEFORALL_IGDB_TOKEN");
  }
  const url = `https://id.twitch.tv/oauth2/token?client_id=${encodeURIComponent(id)}&client_secret=${encodeURIComponent(secret)}&grant_type=client_credentials`;
  const res = await fetch(url, { method: "POST" });
  const json = await res.json();
  if (!json.access_token) throw new Error(`Twitch a refusé : ${JSON.stringify(json)}`);
  const until = new Date(Date.now() + json.expires_in * 1000);
  const set = (k, v) => {
    const re = new RegExp(`^\s*${k}\s*=.*$`, "m");
    text = re.test(text) ? text.replace(re, `${k}=${v}`) : text.replace(/\s*$/, `\n${k}=${v}\n`);
  };
  set("ONEFORALL_IGDB_TOKEN", json.access_token);
  set("ONEFORALL_IGDB_TOKEN_EXPIRES", until.toISOString().slice(0, 10));
  fs.writeFileSync(envFile, text);
  process.env.ONEFORALL_IGDB_TOKEN = json.access_token;
  console.log(`✔ Nouveau jeton IGDB écrit dans .env.local (valable jusqu'au ${until.toLocaleDateString("fr-FR")}).`);
  return json.access_token;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  refreshIgdbToken({ auto: process.argv.includes("--auto") }).catch((e) => {
    console.error(`✖ ${e.message}`);
    process.exit(1);
  });
}
