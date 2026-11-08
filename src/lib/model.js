// Types, statuts, regroupement, tri et formatage.

export const KINDS = [
  { id: "all", label: "Tout" },
  { id: "film", label: "Films" },
  { id: "series", label: "Séries" },
  { id: "anime", label: "Animés" },
  { id: "game", label: "Jeux" },
  { id: "software", label: "Logiciels" },
  { id: "book", label: "Livres" },
  { id: "audio", label: "Audio" },
  { id: "other", label: "Autres" },
];

export const STATUS_FILTERS = [
  { id: "all", label: "Tous" },
  { id: "active", label: "En cours" },
  { id: "done", label: "Terminés" },
  { id: "error", label: "Problèmes" },
];

export const SORTS = [
  { id: "added", label: "Date d'ajout" },
  { id: "completed", label: "Date de fin" },
  { id: "name", label: "Nom" },
  { id: "size", label: "Taille" },
  { id: "progress", label: "Progression" },
  { id: "speed", label: "Vitesse" },
];

export const STATUS_LABEL = {
  downloading: "Téléchargement",
  stalled: "En attente de sources",
  seeding: "Terminé",
  done: "En pause",
  paused: "En pause",
  queued: "En file",
  checking: "Vérification",
  error: "Fichiers manquants",
};

const ACTIVE = new Set(["downloading", "stalled", "queued", "checking", "paused"]);
export const isActive = (s) => ACTIVE.has(s);
export const isDone = (s) => s === "seeding" || s === "done";
/** En pause : arrêté par l'utilisateur, qu'il soit terminé (« done ») ou non. */
export const isPaused = (s) => s === "paused" || s === "done";
/** Types qu'on peut lire (double-clic = lecture) */
export const PLAYABLE = new Set(["film", "series", "anime", "audio"]);
export const KIND_LABEL = { film: "Film", series: "Série", anime: "Animé", game: "Jeu", software: "Logiciel", book: "Livre", audio: "Audio", other: "Autre" };

/** Texte d'un torrent partiel : « 3/170 ép. » ou « 12 % ». */
export function partialLabel(g) {
  const withCounts = g.items.filter((i) => i.files_total != null);
  if (withCounts.length === g.items.length && withCounts.length) {
    const sel = withCounts.reduce((s, i) => s + i.files_sel, 0);
    const tot = withCounts.reduce((s, i) => s + i.files_total, 0);
    const unit = g.kind === "series" || g.kind === "anime" ? " ép." : "";
    return `${sel}/${tot}${unit}`;
  }
  const done = g.items.reduce((s, i) => s + i.completed, 0);
  return `${Math.floor((done / Math.max(1, g.totalSize)) * 100)} %`;
}

/** Statut d'un groupe : le plus « important » de ses éléments. */
function groupStatus(items) {
  const order = ["error", "downloading", "checking", "stalled", "queued", "paused", "seeding", "done"];
  let best = order.length - 1;
  for (const it of items) best = Math.min(best, order.indexOf(it.status));
  return order[best];
}

export function buildGroups(items, grouped) {
  const map = new Map();
  for (const it of items) {
    const key = grouped ? it.group_key : it.hash;
    let g = map.get(key);
    if (!g) map.set(key, (g = { key, items: [] }));
    g.items.push(it);
  }
  return [...map.values()].map((g) => {
    const its = g.items.sort((a, b) => (a.season ?? 0) - (b.season ?? 0) || (a.episode ?? 0) - (b.episode ?? 0) || b.added_on - a.added_on);
    const size = its.reduce((s, i) => s + i.size, 0);
    const done = its.reduce((s, i) => s + i.size * i.progress, 0);
    const first = its[0];
    const seasons = [...new Set(its.map((i) => i.season).filter((s) => s != null))];
    return {
      key: g.key,
      artKey: first.group_key,
      items: its,
      totalSize: its.reduce((s, i) => s + i.total_size, 0),
      partial: its.some((i) => i.total_size > i.size * 1.001),
      title: first.title,
      kind: first.kind,
      year: first.year,
      quality: first.quality,
      tags: first.tags,
      seasons,
      size,
      progress: size ? done / size : Math.min(...its.map((i) => i.progress)),
      status: groupStatus(its),
      added_on: Math.max(...its.map((i) => i.added_on)),
      completion_on: Math.max(...its.map((i) => i.completion_on)),
      dlspeed: its.reduce((s, i) => s + i.dlspeed, 0),
      upspeed: its.reduce((s, i) => s + i.upspeed, 0),
      eta: Math.max(...its.map((i) => (isActive(i.status) ? i.eta : 0))),
    };
  });
}

const collator = new Intl.Collator("fr", { sensitivity: "base", numeric: true });
const SORT_FN = {
  added: (a, b) => a.added_on - b.added_on,
  completed: (a, b) => a.completion_on - b.completion_on,
  name: (a, b) => collator.compare(a.title, b.title),
  size: (a, b) => a.size - b.size,
  progress: (a, b) => a.progress - b.progress,
  speed: (a, b) => a.dlspeed + a.upspeed - (b.dlspeed + b.upspeed),
};

export function sortGroups(groups, sort, dir) {
  const fn = SORT_FN[sort] ?? SORT_FN.added;
  const m = dir === "asc" ? 1 : -1;
  return groups.sort((a, b) => m * fn(a, b) || collator.compare(a.title, b.title));
}

// ---------- formatage ----------

export function fmtSize(b) {
  if (!b || b < 0) return "0 o";
  const u = ["o", "Ko", "Mo", "Go", "To"];
  const i = Math.min(u.length - 1, Math.floor(Math.log(b) / Math.log(1024)));
  return `${(b / 1024 ** i).toFixed(i >= 3 ? 1 : 0).replace(".", ",")} ${u[i]}`;
}

export const fmtSpeed = (b) => `${fmtSize(b)}/s`;

const rtf = new Intl.RelativeTimeFormat("fr", { numeric: "auto" });
export function fmtAgo(ts) {
  if (!ts || ts <= 0) return "—";
  const s = ts - Date.now() / 1000;
  const abs = Math.abs(s);
  if (abs < 60) return "à l'instant";
  if (abs < 3600) return rtf.format(Math.round(s / 60), "minute");
  if (abs < 86400) return rtf.format(Math.round(s / 3600), "hour");
  if (abs < 86400 * 30) return rtf.format(Math.round(s / 86400), "day");
  if (abs < 86400 * 365) return rtf.format(Math.round(s / (86400 * 30)), "month");
  return rtf.format(Math.round(s / (86400 * 365)), "year");
}

export function fmtDate(ts) {
  if (!ts || ts <= 0) return "—";
  return new Date(ts * 1000).toLocaleString("fr-FR", { dateStyle: "medium", timeStyle: "short" });
}

export function fmtEta(s) {
  if (!s || s <= 0 || s >= 8640000) return "";
  if (s < 60) return `${s} s`;
  if (s < 3600) return `${Math.round(s / 60)} min`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ${Math.round((s % 3600) / 60)} min`;
  return `${Math.round(s / 86400)} j`;
}

export function subtitle(g) {
  const n = g.items.length;
  if (g.kind === "series" || g.kind === "anime") {
    const s = g.seasons.length === 1 ? `Saison ${g.seasons[0]}` : g.seasons.length > 1 ? `${g.seasons.length} saisons` : "";
    const eps = g.items.filter((i) => i.episode != null).length;
    const e = eps > 1 ? `${eps} épisodes` : n > 1 ? `${n} éléments` : g.items[0].episode != null ? `Épisode ${g.items[0].episode}` : "";
    return [s, e].filter(Boolean).join(" · ");
  }
  if (n > 1) return `${n} éléments`;
  return g.year ? String(g.year) : "";
}

/** Couleur stable dérivée du titre, pour les jaquettes générées. */
export function hueOf(str) {
  let h = 0;
  for (let i = 0; i < str.length; i++) h = (h * 31 + str.charCodeAt(i)) >>> 0;
  return h % 360;
}

/** Position de lecture : 1137634 ms → « 18:57 », 4000000 → « 1:06:40 » */
export function fmtPos(ms) {
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = String(Math.floor((s % 3600) / 60)).padStart(h ? 2 : 1, "0");
  return `${h ? h + ":" : ""}${m}:${String(s % 60).padStart(2, "0")}`;
}

/** Thèmes de l'app (dark = barre de titre Windows sombre). */
export const THEMES = [
  { id: "light", label: "Clair", icon: "sun", dark: false, sw: ["#f3f4f8", "#0067c0"] },
  { id: "dark", label: "Sombre", icon: "moon", dark: true, sw: ["#10131a", "#3b8eea"] },
  { id: "kawaii", label: "Kawaii", icon: "heart", dark: false, sw: ["#fff0f6", "#ff4d94"] },
  { id: "minuit", label: "Minuit", icon: "moon", dark: true, sw: ["#120d1f", "#a78bfa"] },
  { id: "menthe", label: "Menthe", icon: "leaf", dark: false, sw: ["#eef8f3", "#0f9f6e"] },
];
export const themeOf = (id) => THEMES.find((t) => t.id === id) ?? THEMES[0];
