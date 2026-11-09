// État d'interface partagé : menu contextuel, confirmation, notifications, affiches.
import { forgetArtwork, getArtwork } from "./api.js";

export const ui = $state({
  menu: null, // { x, y, items }
  confirm: null, // { title, text, ok, danger, resolve }
  toast: null,
  studio: null, // chemin de la vidéo ouverte dans le studio d'extraits
  roms: [], // ROM à lancer (fenêtre « Jouer »)
  diskMap: null, // { mount, used } : fenêtre « Analyser le stockage »
  artPick: null,
  rename: null, // { key, title } : fenêtre « Vrai nom du jeu » // { key, kind, title, year } : fenêtre « Choisir l'image »
});

export function openMenu(e, items) {
  e.preventDefault();
  e.stopPropagation();
  ui.menu = { x: e.clientX, y: e.clientY, items };
}

/** Studio d'extraits sur une vidéo locale (fichier, ou dossier : sa plus grosse vidéo) */
export function openStudio(path) {
  ui.studio = path;
}

export function confirmAsk(title, text, ok = "Confirmer", danger = false) {
  return new Promise((resolve) => (ui.confirm = { title, text, ok, danger, resolve }));
}

let toastTimer;
/** Notification en bas de l'écran, avec boutons d'action facultatifs. */
export function flash(msg, actions = []) {
  ui.toast = { msg, actions };
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (ui.toast = null), actions.length ? 9000 : 3500);
}

export async function copy(text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const t = Object.assign(document.createElement("textarea"), { value: text });
    document.body.append(t);
    t.select();
    document.execCommand("copy");
    t.remove();
  }
  flash("Copié dans le presse-papiers");
}

// ---------- affiches ----------
// arts[clé] : undefined = pas encore demandé, null = rien trouvé, { url, fallback } = trouvé

export const arts = $state({});
const queue = [];
const pending = new Set();
let running = 0;

export function wantArt(g) {
  if (g.artKey in arts || pending.has(g.artKey)) return;
  pending.add(g.artKey);
  queue.push(g);
  pump();
}

function pump() {
  while (running < 4 && queue.length) {
    const g = queue.shift();
    running++;
    getArtwork(g.artKey, g.kind, g.artTitle ?? g.title, g.year)
      .then((a) => (arts[g.artKey] = a ?? null))
      .catch(() => {})
      .finally(() => {
        running--;
        pending.delete(g.artKey);
        pump();
      });
  }
}

/** Fenêtre « Choisir l'image » pour un groupe (affiche, jaquette…) */
export function pickArt(g) {
  ui.artPick = { key: g.artKey, kind: g.kind, title: g.artTitle ?? g.title, year: g.year ?? null };
}

export async function refreshArt(key) {
  await forgetArtwork(key);
  delete arts[key];
}

// ---------- calendrier de diffusion (séries / animés) ----------
import { getSchedule } from "./api.js";

// schedules[clé] : undefined = pas demandé, null = rien trouvé, sinon { show, status, next, episodes }
export const schedules = $state({});
const schedPending = new Set();
const schedQueue = [];
let schedRunning = 0;

/** Demande le calendrier d'un groupe série/animé (titres variés : « JoJos » / « JoJo's »…). */
export function wantSchedule(g) {
  if (!(g.kind === "series" || g.kind === "anime")) return;
  if (g.artKey in schedules || schedPending.has(g.artKey)) return;
  schedPending.add(g.artKey);
  const titles = [...new Set(g.items.map((i) => i.title))].slice(0, 4);
  schedQueue.push({ key: g.artKey, titles, kind: g.kind });
  pumpSched();
}

function pumpSched() {
  while (schedRunning < 2 && schedQueue.length) {
    const job = schedQueue.shift();
    schedRunning++;
    getSchedule(job.titles, job.kind)
      .then((s) => (schedules[job.key] = s ?? null))
      .catch(() => {})
      .finally(() => {
        schedRunning--;
        schedPending.delete(job.key);
        pumpSched();
      });
  }
}

/** Date de diffusion d'un épisode, en millisecondes. */
export const airTime = (ep) => (ep?.at ? ep.at * 1000 : ep?.airstamp ? Date.parse(ep.airstamp) : 0);

/** « dans 19 h », « dans 3 j (sam.) », « sorti il y a 2 j » */
export function fmtAir(ms, now = Date.now()) {
  const d = ms - now;
  const abs = Math.abs(d);
  const min = Math.round(abs / 60000);
  const h = Math.round(abs / 3600000);
  const days = Math.round(abs / 86400000);
  const txt = min < 60 ? `${min} min` : h < 24 ? `${h} h` : `${days} j`;
  if (d > 0) {
    const day = abs > 86400000 ? ` (${new Date(ms).toLocaleDateString("fr-FR", { weekday: "short" })})` : "";
    return `dans ${txt}${day}`;
  }
  return `sorti il y a ${txt}`;
}

/** Équipe de sortie d'un nom de torrent : « [Trix] … » ou « …-Tsundere-Raws » */
export function releaseGroup(name) {
  const lead = name.match(/^\s*\[([^\]]+)\]/);
  if (lead) return lead[1];
  const tail = name.replace(/\.(mkv|mp4|avi)$/i, "").match(/-([A-Za-z0-9]+(?:-Raws)?)$/);
  return tail ? tail[1] : null;
}

/**
 * Épisodes de la saison en cours, numérotés comme les fichiers de l'utilisateur.
 * AniList recommence parfois à 1 là où les sorties continuent (« S06E08 » = épisode 7 d'AniList) :
 * si on possède un numéro plus grand que le dernier épisode sorti, on en déduit le décalage.
 */
export function alignEpisodes(sched, owned, season, now = Date.now()) {
  if (!sched) return [];
  let eps = sched.episodes.filter((e) => e.season === 0 || e.season === (season || 1));
  const aired = eps.filter((e) => airTime(e) && airTime(e) <= now).map((e) => e.number);
  const lastAired = aired.length ? Math.max(...aired) : 0;
  const maxOwned = owned.size ? Math.max(...owned) : 0;
  const offset = sched.source === "anilist" && lastAired && maxOwned > lastAired ? maxOwned - lastAired : 0;
  return offset ? eps.map((e) => ({ ...e, number: e.number + offset })) : eps;
}

// ---------- vrai nom des jeux hors torrent (ROM trouvées, téléchargements directs) ----------
// carte → { name, show } : name sert à chercher la jaquette et les infos ; show = l'afficher sur la carte
export const gameNames = $state(JSON.parse(localStorage.getItem("oneforall.gameNames") ?? "{}"));
export function setGameName(key, name, show) {
  if (name) gameNames[key] = { name, show };
  else delete gameNames[key];
  localStorage.setItem("oneforall.gameNames", JSON.stringify(gameNames));
}
/** Applique le nom choisi à une carte : clé d'image propre à ce nom */
export function withGameName(g) {
  const o = gameNames[g.key];
  if (!o) return g;
  return { ...g, artKey: `game:named:${o.name.toLowerCase()}`, artTitle: o.name, origTitle: g.title, title: o.show ? o.name : g.title };
}
export function renameGame(g) {
  ui.rename = { key: g.key, title: g.origTitle ?? g.title };
}
