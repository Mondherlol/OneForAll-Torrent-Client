// Recherche intelligente : lit le nom des releases (langue, source, codec, résolution, saison complète…),
// les note selon les préférences et propose la meilleure, avec la raison en clair.

export const SMART_DEFAULTS = { lang: "multi", res: "1080", codec: "x265", bluray: true };

const has = (rx, s) => rx.test(s);

/** Langues lues dans un texte libre (description Nyaa, liste des fichiers) */
export function langsFromText(text) {
  const t = text ?? "";
  const FR = String.raw`\b(?:fr|fre|fra|french|fran[cç]ais|vf|vff|vostfr|truefrench)\b`;
  const subs =
    /\bvostfr\b|\bsubfrench\b/i.test(t) ||
    new RegExp(String.raw`(?:sous[- ]?titres?|subs?|subtitles?|st)\s*[:=]?[^\n]{0,80}?` + FR, "i").test(t) ||
    /\.(?:fr|fre|fra|french)\.(?:ass|srt|ssa|vtt)\b/i.test(t);
  const audio =
    /\b(?:vff|vfq|vfi|vf2|truefrench)\b/i.test(t) ||
    /\bmulti\b(?![- ]?subs?)/i.test(t) ||
    new RegExp(String.raw`(?:audio|langues?|languages?|dub(?:bing)?|doublage|piste)\s*[:=]?[^\n]{0,80}?` + String.raw`\b(?:fr|fre|fra|french|fran[cç]ais|vf|vff)\b`, "i").test(t);
  return { audio, subs };
}

/** « Frieren.S01.MULTi.1080p.BluRay.x265-TEAM » → { lang, res, source, codec, pack, season… } */
export function parseRelease(title, extra = "") {
  const t = ` ${title.replace(/[._]/g, " ")} `;
  const all = `${t}\n${extra}`;

  // Langue (titre + description si vérifiée)
  let lang = "unknown";
  const vf = has(/\b(VFF|VFQ|VFI|VF2|VF|TRUEFRENCH|FRENCH)\b/i, t);
  const vost = has(/\b(VOSTFR|SUBFRENCH)\b/i, t);
  if (has(/\bMULTI\b(?![- ]?subs?)/i, t) || (vf && vost)) lang = "multi"; // « VOSTFR/FRENCH-VF » = les deux
  else if (vf) lang = "vf";
  else if (vost) lang = "vostfr";
  let verified = false;
  if (extra) {
    const l = langsFromText(extra);
    if (l.audio && l.subs) lang = "multi";
    else if (l.audio && lang !== "multi") lang = lang === "vostfr" ? "multi" : "vf";
    else if (l.subs && lang === "unknown") lang = "vostfr";
    else if (l.subs && lang === "vf") lang = "multi";
    verified = l.audio || l.subs;
  }

  const res = has(/\b(2160p|4K|UHD)\b/i, all) ? 2160 : has(/\b1080[pi]\b/i, all) ? 1080 : has(/\b720p\b/i, all) ? 720 : has(/\b(480p|576p|DVDRip|XviD)\b/i, all) ? 480 : 0;
  const source = has(/\b(CAM|HDCAM|TS|TELESYNC|TC|HDTS)\b/, t)
    ? "cam"
    : has(/\bREMUX\b/i, t)
      ? "remux"
      : has(/\b(BLU-?RAY|BDRIP|BRRIP|BD-?RIP|BDMV|BD)\b/i, all)
        ? "bluray"
        : has(/\bWEB-?DL\b/i, all)
          ? "webdl"
          : has(/\b(WEB-?RIP|WEB)\b/i, all)
            ? "webrip"
            : has(/\b(HDTV|TVRIP)\b/i, all)
              ? "hdtv"
              : "";
  // « x.265 » devient « x 265 » une fois les points remplacés
  const codec = has(/\b([xh] ?265|HEVC)\b/i, all) ? "x265" : has(/\bAV1\b/i, all) ? "av1" : has(/\b([xh] ?264|AVC)\b/i, all) ? "x264" : "";

  // Contenu : épisode seul, saison complète, plusieurs saisons / intégrale
  let pack = "", season = null, seasons = null, episode = false;
  // « S01-S05 », « S01 + S02 »
  const multi = t.match(/\bS(\d{1,2})\s?(?:-|\+|&)\s?S?(\d{1,2})\b(?!\s?E\d)/i) ?? t.match(/\bSaisons?\s(\d{1,2})\s?(?:-|à|a)\s?(\d{1,2})\b/i);
  if (has(/\bS\d{1,2}\s?E\d{1,4}\b/i, t) && !has(/\bS\d{1,2}\s?E\d{1,4}\s?-\s?E?\d{1,4}\b/i, t)) episode = true;
  else if (multi && +multi[2] > +multi[1]) ((pack = "multi"), (seasons = [+multi[1], +multi[2]]));
  else if (has(/\b(INT[EÉ]GRALE|COMPLETE SERIES|THE COMPLETE|COLLECTION)\b/i, t)) pack = "multi";
  else {
    const s = t.match(/\bS(\d{1,2})\b(?!\s?E\d)/i) ?? t.match(/\bSaison\s?(\d{1,2})\b/i) ?? t.match(/\bSeason\s?(\d{1,2})\b/i);
    if (s) ((pack = "season"), (season = +s[1]));
    // Animés (Nyaa) : « (01-12) », « [Batch] », « 01 ~ 24 »
    const rg = t.match(/[\s([](\d{1,4})\s?(?:-|~)\s?(\d{1,4})[\s)\]]/);
    if (!pack && (has(/\b(BATCH|COMPLETE)\b/i, t) || (rg && +rg[2] > +rg[1] && !(+rg[1] > 1900)))) (pack = "season");
    else if (!pack && has(/\s-\s\d{1,4}(v\d)?\s/, t)) episode = true;
    if (pack && has(/\b(COMPLETE|INT[EÉ]GRALE)\b/i, t) && !s) pack = "multi";
  }
  return { lang, verified, res, source, codec, pack, season, seasons, episode };
}

const GiB = 1024 ** 3;

/** Note d'un résultat (plus c'est haut, mieux c'est) + raisons affichées */
export function scoreRelease(r, info, { prefs = SMART_DEFAULTS, kind, wantPack = true } = {}) {
  let s = 0;
  const why = [];
  const video = kind === "film" || kind === "series" || kind === "anime";

  // Sources (seeders) : vite utile, puis plafonne
  const seeds = Math.max(0, r.seeders ?? 0);
  if (r.seeders === 0) s -= 80;
  else if (r.seeders >= 0) s += Math.min(30, 15 * Math.log10(1 + seeds)) - (seeds < 3 ? 20 : 0);
  if (r.trusted) s += 4;

  if (video) {
    // Langue
    const L = { multi: 40, vf: 30, vostfr: 22, unknown: 0 };
    if (prefs.lang === "vf") ((L.vf = 40), (L.multi = 36), (L.vostfr = 8));
    if (prefs.lang === "vostfr") ((L.vostfr = 40), (L.multi = 36), (L.vf = 5));
    if (prefs.lang === "any") ((L.multi = 6), (L.vf = 4), (L.vostfr = 4));
    s += L[info.lang];
    if (info.lang === "unknown" && r.source === "nyaa" && prefs.lang !== "any") s -= 12; // probablement VO + sous-titres anglais
    // Résolution
    const target = +prefs.res;
    s += info.res === target ? 25 : info.res === 0 ? 0 : info.res > target ? 6 : info.res === 720 ? 8 : -15;
    // Source
    s += { cam: -150, remux: prefs.bluray ? 4 : -4, bluray: prefs.bluray ? 22 : 10, webdl: 12, webrip: 8, hdtv: 0, "": 0 }[info.source];
    // Codec
    s += prefs.codec === "x265" ? { x265: 16, av1: 13, x264: 2, "": 0 }[info.codec] : { x265: 6, av1: 5, x264: 6, "": 0 }[info.codec];
    // Taille : un film 1080p x265 fait 2-10 Go ; trop petit = faux ou très dégradé, énorme = remux
    if (kind === "film" && r.size > 0) {
      const g = r.size / GiB;
      if (g < 0.6) s -= 25;
      else if (g > 25 && prefs.codec === "x265") s -= 12;
      else if (g > 15 && prefs.codec === "x265") s -= 5;
    }
    // Séries / animés : la saison entière plutôt que l'épisode
    if (kind !== "film" && wantPack) s += info.pack === "season" ? 35 : info.pack === "multi" ? 24 : info.episode ? -18 : 0;
  } else if (kind === "game") {
    // Jeux : repack FitGirl (le plus léger) > Online-Fix (jeu en ligne) > le reste
    s += { fitgirl: 24, "online-fix": 14 }[r.source] ?? 0;
    if (r.source === "fitgirl" || r.source === "online-fix") s += 25; // pas de seeders connus : source fiable
  }

  // Raisons, dans l'ordre où on les lit
  if (video) {
    if (info.pack === "season") why.push(info.season != null ? `Saison ${info.season} complète` : "Pack complet");
    else if (info.pack === "multi") why.push(info.seasons ? `Saisons ${info.seasons[0]} à ${info.seasons[1]}` : "Intégrale");
    if (info.lang !== "unknown") why.push({ multi: "MULTI (VF + VOSTFR)", vf: "VF", vostfr: "VOSTFR" }[info.lang] + (info.verified ? " ✓" : ""));
    if (info.res) why.push(info.res === 2160 ? "4K" : `${info.res}p`);
    if (info.source && info.source !== "cam") why.push({ remux: "Remux BluRay", bluray: "BluRay", webdl: "WEB-DL", webrip: "WEBRip", hdtv: "HDTV" }[info.source]);
    if (info.codec) why.push(info.codec === "x265" ? "x265 (léger)" : info.codec.toUpperCase());
  } else if (kind === "game") {
    if (r.source === "fitgirl") why.push("Repack FitGirl (téléchargement réduit)");
    if (r.source === "online-fix") why.push("Online-Fix (jouable en ligne)");
  }
  return { score: s, why };
}

/** Tous les mots cherchés sont dans le titre */
export function matchesAll(title, qWords, words) {
  const t = words(title);
  return qWords.length > 0 && qWords.every((w) => t.includes(w) || (w.length > 3 && t.some((x) => x.startsWith(w))));
}
