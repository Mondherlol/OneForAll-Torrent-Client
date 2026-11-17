<script>
  import { fly } from "svelte/transition";
  import { untrack } from "svelte";
  import { open as pickFolder } from "@tauri-apps/plugin-dialog";
  import Icon from "./Icon.svelte";
  import Compress from "./Compress.svelte";
  import DlOptions from "./DlOptions.svelte";
  import { addDownloads, downloadAction, mediaInfo, moveDownloads, openPath, openUrl, reveal } from "./api.js";
  import { KIND_LABEL, fmtAgo, fmtEta, fmtSize, fmtSpeed } from "./model.js";
  import { arts, confirmAsk, flash, openMenu, openStudio, pickArt, ui, wantArt } from "./ui.svelte.js";
  import { dlExtract, dlSetKind, emuDetect, hostDownload, isHostLink, readBook } from "./api.js";
  import ConsoleBadge from "./ConsoleBadge.svelte";
  import { emuById, play } from "./emu.svelte.js";

  /** downloads : liste de get_downloads ; onrefresh : recharge la liste tout de suite ; prefill : lien collé dans la recherche */
  let { downloads, onrefresh, prefill = null } = $props();

  let links = $state("");
  let inputEl = $state();
  $effect(() => {
    if (prefill?.text) {
      links = prefill.text;
      setTimeout(() => inputEl?.focus(), 50);
    }
  });
  const FILTERS = [
    { id: "all", label: "Tous" },
    { id: "active", label: "En cours" },
    { id: "done", label: "Terminés" },
    { id: "error", label: "Problèmes" },
  ];
  let filter = $state("all");
  const isActive = (d) => d.st === "running" || d.st === "queued" || d.st === "paused";
  const counts = $derived({
    all: downloads.length,
    active: downloads.filter(isActive).length,
    done: downloads.filter((d) => d.st === "done").length,
    error: downloads.filter((d) => d.st === "error").length,
  });
  const shown = $derived(downloads.filter((d) => filter === "all" || (filter === "active" ? isActive(d) : d.st === filter)));
  const speed = $derived(downloads.reduce((s, d) => s + d.speed, 0));
  const running = $derived(downloads.some((d) => d.st === "running" || d.st === "queued"));
  const pausedAny = $derived(downloads.some((d) => d.st === "paused"));

  $effect(() => {
    for (const d of downloads) if (!d.media) wantArt({ artKey: d.art_key, kind: d.kind, title: d.title, year: d.year });
  });

  // Vidéos / musiques des sites (yt-dlp) : même liste que ytdl.rs
  const MEDIA_HOSTS = /(^|\.)(youtube\.com|youtu\.be|twitter\.com|x\.com|instagram\.com|tiktok\.com|facebook\.com|fb\.watch|vimeo\.com|dailymotion\.com|dai\.ly|twitch\.tv|reddit\.com|redd\.it|soundcloud\.com|bilibili\.com|streamable\.com|threads\.net|bsky\.app|kick\.com|pinterest\.(com|fr)|snapchat\.com|bandcamp\.com|open\.spotify\.com)$/i;
  const isMedia = (u) => {
    try {
      return MEDIA_HOSTS.test(new URL(u).hostname);
    } catch {
      return false;
    }
  };
  const isSpotify = (u) => /^https?:\/\/open\.spotify\.com\//i.test(u);
  const hasSpotify = $derived(links.split(/\s+/).some(isSpotify));

  // Un seul lien de vidéo : on lit titre, durée, qualités et aperçu dès qu'il est collé (prêt pour les options)
  const isList = (u) => /[?&]list=/.test(u) && !/[?&]v=|youtu\.be\//.test(u);
  const single = $derived.by(() => {
    const l = links.trim().split(/\s+/).filter(Boolean);
    return l.length === 1 && isMedia(l[0]) && !isSpotify(l[0]) && !isList(l[0]) ? l[0] : null;
  });
  let info = $state(null);
  let infoFor = $state(null);
  let infoErr = $state("");
  // L'effet ne dépend que du lien : avant, il se relançait (il lit et écrit infoFor) et son
  // nettoyage annulait la lecture prévue → « Lecture de la vidéo… » sans fin.
  $effect(() => {
    const u = single;
    const t = untrack(() => {
      if (u === infoFor) return;
      info = null;
      infoErr = "";
      infoFor = u;
      if (!u) return;
      return setTimeout(() => {
        mediaInfo(u)
          .then((r) => infoFor === u && (info = r))
          .catch((e) => infoFor === u && (infoErr = String(e)));
      }, 300);
    });
    return () => clearTimeout(t);
  });

  // Sous le titre : le site d'origine, la vidéo source (compression / extrait) ou le nom du fichier
  function origin(d) {
    if (d.media === "compress") return `Depuis ${d.url.split(/[\\/]/).pop()}`;
    if (d.media === "clip") {
      try {
        const s = JSON.parse(d.url);
        return `Depuis ${s.path.split(/[\\/]/).pop()}${s.segs.length > 1 ? ` · ${s.segs.length} passages` : ""}`;
      } catch {
        return "";
      }
    }
    if (d.media) {
      try {
        return new URL(d.url).hostname.replace(/^(www|m)\./, "");
      } catch {
        return d.url;
      }
    }
    return d.name;
  }

  let compressing = $state(null);
  // ROM téléchargée (.nds, .gba… ou .zip / .7z qui en contient une) → bouton « Jouer »
  const ROMISH = /\.(zip|7z|nds|dsi|gba|gbc?|3ds|cia|cci|cso|chd|pbp|iso|cue|rvz|wbfs|gcz|nsp|xci)$/i;
  let roms = $state({}); // chemin → ROM détectée (null = pas un jeu)
  // La ROM extraite si elle existe, sinon le fichier téléchargé (archive comprise)
  const romPath = (d) => d.rom || d.path;
  const romOf = (d) => roms[romPath(d)];
  $effect(() => {
    for (const d of downloads) {
      const p = romPath(d);
      if (d.st !== "done" || !p || !(d.rom || ROMISH.test(d.name)) || p in roms) continue;
      roms[p] = null;
      emuDetect(p).then((r) => (roms[p] = r.emu ? r : null)).catch(() => {});
    }
  });
  const isGame = (d) => d.kind === "game" || !!d.platform || !!romOf(d);
  const isArchive = (d) => /\.(zip|7z)$/i.test(d.name);
  let extracting = $state(null);
  async function extract(d) {
    extracting = d.id;
    try {
      const rom = await dlExtract(d.id);
      roms[rom.path] = rom.emu ? rom : null;
      flash(`${rom.name} extrait · zip mis à la corbeille`);
      onrefresh();
    } catch (e) {
      flash(String(e));
    } finally {
      extracting = null;
    }
  }
  // Double-clic : un jeu se lance, le reste s'ouvre
  const dbl = (d) => (d.st === "done" && romOf(d) ? play(romOf(d)) : openItem(d));
  const artFor = (d) => ({ artKey: d.art_key, kind: d.kind, title: d.title, year: d.year });

  const isVideoFile = (d) => d.st === "done" && /\.(mp4|mkv|webm|mov|avi|m4v)$/i.test(d.name);

  // Télécharger : vidéos des sites → fenêtre d'options (qualité, MP3, passage) ; fichiers et Spotify → direct
  let options = $state(null);
  function add() {
    let urls = links.split(/\s+/).filter((l) => /^https?:\/\//i.test(l));
    if (!urls.length) return flash("Colle au moins un lien https://…");
    // Hébergeurs (1fichier, DataNodes…) : la fenêtre qui fait les clics s'en occupe
    const hosts = urls.filter(isHostLink);
    if (hosts.length) {
      for (const u of hosts) hostDownload(u).catch((e) => flash(String(e)));
      urls = urls.filter((u) => !hosts.includes(u));
      links = "";
      if (!urls.length) return;
    }
    const videos = urls.filter((u) => isMedia(u) && !isSpotify(u));
    if (videos.length) options = { urls: videos, others: urls.filter((u) => !videos.includes(u)) };
    else start(urls, "max", null);
  }
  async function start(urls, format, section) {
    try {
      const ids = await addDownloads(urls, format, section);
      links = "";
      flash(ids.length > 1 ? `${ids.length} téléchargements ajoutés` : "Téléchargement ajouté");
      onrefresh();
    } catch (e) {
      flash(String(e));
    }
  }
  async function confirmOptions({ format, section }) {
    const { urls, others } = options;
    options = null;
    if (others.length) await addDownloads(others, "max", null).catch(() => {});
    start(urls, format, section);
  }

  async function act(items, action) {
    try {
      await downloadAction(items.map((d) => d.id), action);
      onrefresh();
    } catch (e) {
      flash(String(e));
    }
  }

  async function remove(items, withFile) {
    const n = items.length;
    const what = n === 1 ? `« ${items[0].name} »` : `${n} téléchargements`;
    const size = items.reduce((s, d) => s + Math.max(0, d.done), 0);
    const allDone = items.every((d) => d.st === "done");
    const ok = await confirmAsk(
      withFile ? `Supprimer ${what} ${n === 1 ? "et le fichier" : "et leurs fichiers"} ?` : `Retirer ${what} de la liste ?`,
      withFile
        ? `${fmtSize(size)} seront effacés (corbeille).`
        : allDone
          ? n === 1 ? "Le fichier reste sur le disque." : "Les fichiers restent sur le disque."
          : items.some((d) => d.st === "done") ? "Les fichiers terminés restent sur le disque, les fichiers partiels (.part) sont effacés." : "Le fichier partiel (.part) est effacé.",
      withFile ? "Supprimer" : "Retirer",
      withFile,
    );
    if (!ok) return;
    const ids = new Set(items.map((d) => d.id));
    sel = new Set([...sel].filter((id) => !ids.has(id)));
    act(items, withFile ? "delete_files" : "delete");
  }

  async function moveTo(items) {
    const d = items[0];
    const dir = await pickFolder({ directory: true, title: items.length > 1 ? `Déplacer ${items.length} téléchargements vers…` : `Déplacer « ${d.name} » vers…`, defaultPath: d.dir || undefined });
    if (typeof dir !== "string") return;
    try {
      await moveDownloads(items.map((x) => x.id), dir);
      flash(`Déplacé vers ${dir}`);
      onrefresh();
    } catch (e) {
      flash(String(e));
    }
  }

  async function clearDone() {
    const done = downloads.filter((d) => d.st === "done");
    if (done.length) await act(done, "delete");
  }

  const copy = (t, n = 1) => navigator.clipboard.writeText(t).then(() => flash(n > 1 ? `${n} liens copiés` : "Lien copié"));
  const copyLinks = (items) => {
    const urls = items.filter((d) => d.media !== "compress" && d.media !== "clip").map((d) => d.url);
    if (urls.length) copy(urls.join("\n"), urls.length);
  };
  // Terminé : on ouvre le fichier. Sinon : la vidéo d'origine (pas encore de fichier), ou le dossier.
  const web = (d) => d.media && d.media !== "compress" && d.media !== "clip";
  const openItem = (d) =>
    (d.st === "done" ? (d.kind === "book" ? readBook(d.path) : openPath(d.path)) : web(d) ? openUrl(d.url) : reveal(d.path, d.dir)).catch((e) => flash(String(e)));

  const isBusy = (d) => d.st === "running" || d.st === "queued";
  const kindMenu = (items) => ({
    label: "Type",
    icon: "tag",
    sub: Object.entries(KIND_LABEL).map(([k, label]) => ({
      label,
      checked: items.every((d) => d.kind === k),
      run: () => Promise.all(items.map((d) => dlSetKind(d.id, k))).then(onrefresh).catch((err) => flash(String(err))),
    })),
  });

  // Plusieurs éléments sélectionnés : actions groupées
  function multiMenu(e, items) {
    openMenu(e, [
      items.some(isBusy) && { label: "Pause", icon: "pause", run: () => act(items.filter(isBusy), "pause") },
      items.some((d) => d.st === "paused") && { label: "Reprendre", icon: "play", run: () => act(items.filter((d) => d.st === "paused"), "resume") },
      items.some((d) => d.st === "error") && { label: "Réessayer", icon: "refresh", run: () => act(items.filter((d) => d.st === "error"), "resume") },
      { label: "Déplacer…", icon: "move", run: () => moveTo(items) },
      { label: "Copier les liens", icon: "copy", run: () => copyLinks(items) },
      items.every((d) => !d.media) && kindMenu(items),
      "-",
      { label: `Retirer de la liste (${items.length})`, icon: "close", run: () => remove(items, false) },
      items.some((d) => d.done > 0) && { label: "Supprimer avec les fichiers", icon: "trash", danger: true, run: () => remove(items, true) },
    ]);
  }

  // ---------- sélection (comme les onglets torrents) ----------
  let layout = $state(localStorage.getItem("oneforall.dlView") === "cards" ? "cards" : "list");
  $effect(() => localStorage.setItem("oneforall.dlView", layout));
  let sel = $state(new Set());
  let anchor = null;
  const selItems = $derived(shown.filter((d) => sel.has(d.id)));
  // Ce qui disparaît (filtre, retiré) sort de la sélection
  $effect(() => {
    const ids = new Set(shown.map((d) => d.id));
    untrack(() => {
      if ([...sel].some((id) => !ids.has(id))) sel = new Set([...sel].filter((id) => ids.has(id)));
    });
  });

  let skipClick = false;
  function itemClick(e, d, i) {
    if (skipClick || e.target.closest("button")) return;
    if (e.ctrlKey || e.metaKey) {
      const s = new Set(sel);
      s.has(d.id) ? s.delete(d.id) : s.add(d.id);
      sel = s;
      anchor = i;
    } else if (e.shiftKey && anchor != null) {
      const [a, b] = anchor < i ? [anchor, i] : [i, anchor];
      sel = new Set([...sel, ...shown.slice(a, b + 1).map((x) => x.id)]);
    } else {
      anchor = i;
      if (sel.size) sel = new Set();
    }
  }
  function itemMenu(e, d) {
    if (sel.size > 1 && sel.has(d.id)) return multiMenu(e, selItems);
    if (sel.size && !sel.has(d.id)) sel = new Set();
    menu(e, d);
  }

  // Cliquer-glisser → rectangle de sélection. Pas de capture du pointeur (elle volait les clics
  // et doubles-clics des lignes) : on suit la souris sur la fenêtre.
  let band = $state(null);
  let bandBase = new Set();
  function bandDown(e) {
    if (e.button !== 0 || !e.target.closest?.("main")) return;
    if (e.target.closest("button, input, select, textarea, a, label, .add, .bar, .menu, .modal, .dialog, .selbar")) return;
    bandBase = e.ctrlKey || e.shiftKey ? new Set(sel) : new Set();
    band = { x0: e.clientX, y0: e.clientY, x1: e.clientX, y1: e.clientY, moved: false, onItem: !!e.target.closest("[data-id]") };
  }
  function bandMove(e) {
    if (!band) return;
    band.x1 = e.clientX;
    band.y1 = e.clientY;
    if (!band.moved && Math.hypot(band.x1 - band.x0, band.y1 - band.y0) < 6) return;
    band.moved = true;
    window.getSelection()?.removeAllRanges();
    const [l, r] = [Math.min(band.x0, band.x1), Math.max(band.x0, band.x1)];
    const [t, b] = [Math.min(band.y0, band.y1), Math.max(band.y0, band.y1)];
    const s = new Set(bandBase);
    for (const el of document.querySelectorAll(".dls [data-id]")) {
      const rc = el.getBoundingClientRect();
      if (rc.right > l && rc.left < r && rc.bottom > t && rc.top < b) s.add(el.dataset.id);
    }
    sel = s;
  }
  function bandUp() {
    if (!band) return;
    if (band.moved) {
      // Le clic qui suit le relâchement ne doit pas défaire la sélection
      skipClick = true;
      setTimeout(() => (skipClick = false), 0);
    } else if (!band.onItem && !bandBase.size) sel = new Set();
    band = null;
  }

  function onkey(e) {
    if (ui.confirm || ui.menu || options || compressing || document.querySelector(".modal, .dialog")) return;
    const t = e.target;
    if (t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement || t instanceof HTMLSelectElement) return;
    if (e.ctrlKey && e.key.toLowerCase() === "a") {
      e.preventDefault();
      sel = new Set(shown.map((d) => d.id));
    } else if (e.key === "Escape" && sel.size) sel = new Set();
    else if (e.key === "Delete" && selItems.length) remove(selItems, e.shiftKey);
  }

  // Petit logo du site d'origine (vidéos), ou de l'outil (extrait, compression)
  const SITES = [
    [/youtube\.com|youtu\.be/, "yt", "YouTube"],
    [/(^|\.)x\.com|twitter\.com/, "x", "X"],
    [/instagram\.com/, "ig", "Instagram"],
    [/tiktok\.com/, "tt", "TikTok"],
    [/facebook\.com|fb\.watch/, "fb", "Facebook"],
    [/vimeo\.com/, "vm", "Vimeo"],
    [/dailymotion\.com|dai\.ly/, "dm", "Dailymotion"],
    [/twitch\.tv/, "tw", "Twitch"],
    [/reddit\.com|redd\.it/, "rd", "Reddit"],
    [/soundcloud\.com/, "sc", "SoundCloud"],
    [/spotify\.com/, "sp", "Spotify"],
    [/bilibili\.com/, "bl", "Bilibili"],
    [/bsky\.app/, "bs", "Bluesky"],
    [/kick\.com/, "kk", "Kick"],
    [/pinterest\./, "pi", "Pinterest"],
    [/bandcamp\.com/, "bc", "Bandcamp"],
    [/threads\.net/, "th", "Threads"],
    [/streamable\.com/, "st", "Streamable"],
    [/snapchat\.com/, "sn", "Snapchat"],
  ];
  const GLYPH = { x: "𝕏", fb: "f", vm: "v", dm: "d", tw: "t", rd: "r", sc: "☁", bl: "b", bs: "b", kk: "K", pi: "P", bc: "bc", th: "@", st: "s", sn: "S", tt: "♪", web: "" };
  function siteOf(d) {
    if (d.media === "clip") return { id: "clip", label: "Extrait" };
    if (d.media === "compress") return { id: "comp", label: "Compression" };
    if (!d.media) return null;
    let h = "";
    try {
      h = new URL(d.url).hostname;
    } catch {}
    const m = SITES.find(([re]) => re.test(h));
    return m ? { id: m[1], label: m[2] } : { id: "web", label: h.replace(/^(www|m)\./, "") || "Web" };
  }

  function menu(e, d) {
    openMenu(e, [
      d.st === "done" && romOf(d) && { label: `Jouer avec ${emuById(romOf(d).emu)?.name ?? "l'émulateur"}`, icon: "game", run: () => play(romOf(d)) },
      d.st === "done" && isArchive(d) && romOf(d) && (d.rom ? { label: "Supprimer le zip (déjà extrait)", icon: "trash", run: () => extract(d) } : { label: "Extraire le jeu", icon: "layers", run: () => extract(d) }),
      d.st === "done" && { label: "Ouvrir", icon: "open", run: () => openPath(d.path).catch((err) => flash(String(err))) },
      web(d) && { label: "Ouvrir la vidéo d'origine", icon: "open", run: () => openUrl(d.url).catch((err) => flash(String(err))) },
      isVideoFile(d) && { label: "Capturer un extrait…", icon: "scissors", run: () => openStudio(d.path) },
      isVideoFile(d) && { label: "Compresser…", icon: "layers", run: () => (compressing = d) },
      { label: "Ouvrir le dossier", icon: "folder", run: () => reveal(d.path, d.dir).catch((err) => flash(String(err))) },
      "-",
      (d.st === "running" || d.st === "queued") && { label: "Pause", icon: "pause", run: () => act([d], "pause") },
      d.st === "paused" && { label: "Reprendre", icon: "play", run: () => act([d], "resume") },
      d.st === "error" && { label: "Réessayer", icon: "refresh", run: () => act([d], "resume") },
      { label: "Déplacer…", icon: "move", run: () => moveTo([d]) },
      { label: "Copier le lien", icon: "copy", run: () => copy(d.url) },
      !d.media && { label: "Choisir l'image…", icon: "image", run: () => pickArt(artFor(d)) },
      !d.media && kindMenu([d]),
      "-",
      { label: "Retirer de la liste", icon: "close", run: () => remove([d], false) },
      d.done > 0 && { label: "Supprimer avec le fichier", icon: "trash", danger: true, run: () => remove([d], true) },
    ]);
  }

  function statusText(d) {
    if (d.st === "done") return `Terminé ${fmtAgo(d.completion_on)}`;
    if (d.st === "paused") return "En pause";
    if (d.st === "queued") return "En attente de son tour…";
    if (d.st === "error") return d.error || "Erreur";
    if (d.note) return d.note;
    if (d.retrying) return d.error || "Connexion perdue — nouvel essai…";
    if (!d.size || d.size < 0) return d.speed ? fmtSpeed(d.speed) : "Connexion…";
    return "";
  }
</script>

<section class="dls">
  <form class="add" onsubmit={(e) => (e.preventDefault(), add())}>
    <label class="field">
      <Icon name="link" size={15} />
      <input bind:this={inputEl} bind:value={links} placeholder="Lien de fichier, ou de vidéo YouTube, X, Insta, TikTok…" spellcheck="false" />
    </label>
    <button class="btn primary" disabled={!links.trim()}><Icon name="download" size={14} />Télécharger</button>
  </form>
  {#if options}
    <DlOptions urls={options.urls} info={options.urls.length === 1 && options.urls[0] === infoFor ? info : null} infoErr={options.urls.length === 1 ? infoErr : ""} onconfirm={confirmOptions} onclose={() => (options = null)} />
  {/if}
  {#if compressing}
    <Compress d={compressing} onclose={() => (compressing = null)} ondone={() => ((compressing = null), onrefresh())} />
  {/if}
  {#if hasSpotify}
    <p class="sp-note"><Icon name="audio" size={13} />Spotify : chaque morceau est retrouvé sur YouTube et téléchargé en MP3 (playlists et albums compris).</p>
  {/if}

  <div class="bar">
    <div class="seg">
      {#each FILTERS as f}
        <button class:on={filter === f.id} class:warn={f.id === "error" && counts.error > 0} onclick={() => (filter = f.id)}>{f.label}<span class="n">{counts[f.id]}</span></button>
      {/each}
    </div>
    <span class="spacer"></span>
    {#if speed > 0}<span class="speed"><Icon name="down" size={13} />{fmtSpeed(speed)}</span>{/if}
    {#if running}
      <button class="sbtn" onclick={() => act(downloads.filter((d) => d.st === "running" || d.st === "queued"), "pause")}><Icon name="pause" size={12} />Tout en pause</button>
    {:else if pausedAny}
      <button class="sbtn hot" onclick={() => act(downloads.filter((d) => d.st === "paused"), "resume")}><Icon name="play" size={12} />Tout reprendre</button>
    {/if}
    {#if counts.done}<button class="sbtn" onclick={clearDone} title="Retire les terminés de la liste (les fichiers restent)"><Icon name="check" size={12} />Vider les terminés</button>{/if}
    <div class="seg vw">
      <button class:on={layout === "list"} title="Vue liste" onclick={() => (layout = "list")}><Icon name="list" size={14} /></button>
      <button class:on={layout === "cards"} title="Vue cartes" onclick={() => (layout = "cards")}><Icon name="grid" size={14} /></button>
    </div>
  </div>

  {#if !downloads.length}
    <div class="empty">
      <Icon name="down" size={34} />
      <p><b>Aucun téléchargement</b><br />Colle un lien au-dessus : un fichier (reprise auto après une coupure, plusieurs connexions en parallèle) ou une vidéo YouTube, X, Instagram, TikTok, SoundCloud… en HD ou en MP3. Les playlists sont dépliées.</p>
    </div>
  {:else if !shown.length}
    <p class="hint">Rien ici avec ce filtre.</p>
  {:else}
    {#if layout === "list"}
      <ul>
        {#each shown as d, i (d.id)}
          {@const pct = Math.floor(d.progress * 1000) / 10}
          <li data-id={d.id} class="st-{d.st}" class:sel={sel.has(d.id)} class:retry={d.retrying} class:game={isGame(d)} in:fly={{ y: 6, duration: 140 }} onclick={(e) => itemClick(e, d, i)} oncontextmenu={(e) => itemMenu(e, d)} ondblclick={(e) => !e.ctrlKey && !e.shiftKey && dbl(d)}>
            <span class="thumb k-{d.kind}" class:wide={d.media}>
              {@render cover(d)}
              {#if siteOf(d)}<span class="corner">{@render badge(siteOf(d), true)}</span>{/if}
            </span>
            <div class="main">
              <div class="top">
                <span class="title" title={d.name}>{d.title || d.name}</span>
                {@render tags(d)}
              </div>
              {#if d.rom}
                <span class="fname rom" title={d.rom}><Icon name="game" size={12} />{d.rom.split(/[\\/]/).pop()}</span>
              {:else}
                <span class="fname" class:site={d.media} title={d.media === "clip" ? "" : d.url}>{origin(d)}</span>
              {/if}
              {#if d.st !== "done"}
                <div class="pbar"><span style="width:{pct}%"></span></div>
              {/if}
              {@render meta(d, pct)}
            </div>
            <div class="acts">{@render acts(d)}</div>
          </li>
        {/each}
      </ul>
    {:else}
      <div class="cards">
        {#each shown as d, i (d.id)}
          {@const pct = Math.floor(d.progress * 1000) / 10}
          {@const s = siteOf(d)}
          <div data-id={d.id} class="dc st-{d.st}" class:sel={sel.has(d.id)} class:retry={d.retrying} class:game={isGame(d)} role="button" tabindex="-1" in:fly={{ y: 6, duration: 140 }} onclick={(e) => itemClick(e, d, i)} oncontextmenu={(e) => itemMenu(e, d)} ondblclick={(e) => !e.ctrlKey && !e.shiftKey && dbl(d)} onkeydown={() => {}}>
            <div class="cv k-{d.kind}" class:poster={!d.thumb && !d.media}>
              {#if !d.thumb && !d.media && arts[d.art_key]?.url}<img class="blur" src={arts[d.art_key].url} alt="" draggable="false" />{/if}
              {@render cover(d)}
              <span class="cv-tl">
                {#if s}{@render badge(s, false)}{:else}<span class="site s-file" title={KIND_LABEL[d.kind] ?? "Fichier"}><Icon name={d.kind} size={13} /></span>{/if}
              </span>
              <span class="cv-tr">
                {#if d.media}<span class="pill">{d.media_label}</span>{/if}
                {#if d.section_label}<span class="pill"><Icon name="scissors" size={10} />{d.section_label}</span>{/if}
                {#if d.platform}<ConsoleBadge platform={d.platform} small />{/if}
              </span>
              {#if sel.has(d.id)}<span class="tick"><Icon name="check" size={13} /></span>{/if}
              {#if d.st !== "done"}
                <span class="cv-pct">{d.st === "paused" ? "En pause" : d.st === "queued" ? "En attente" : d.st === "error" ? "Erreur" : d.size > 0 ? `${pct.toFixed(0)} %` : "…"}</span>
                <div class="cv-bar"><span style="width:{pct}%"></span></div>
              {/if}
            </div>
            <div class="dc-body">
              <span class="title t2" title={d.name}>{d.title || d.name}</span>
              <span class="fname" class:site={d.media} class:rom={d.rom}>{d.rom ? d.rom.split(/[\\/]/).pop() : origin(d)}</span>
              {@render meta(d, pct)}
            </div>
            <div class="dc-acts">{@render acts(d)}</div>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</section>

{#snippet cover(d)}
  {@const art = arts[d.art_key]}
  {#if isGame(d) && art?.url}<img src={art.url} alt="" draggable="false" />
  {:else if d.thumb}<img src={d.thumb} alt="" referrerpolicy="no-referrer" draggable="false" />
  {:else if art?.url && !d.media}<img src={art.url} alt="" draggable="false" />
  {:else}<span class="ico"><Icon name={d.media === "clip" ? "scissors" : d.media === "compress" ? "layers" : d.media ? (d.media === "mp3" ? "audio" : "film") : d.kind} size={layout === "cards" ? 34 : 18} /></span>{/if}
{/snippet}

{#snippet badge(s, small)}
  <span class="site s-{s.id}" class:sm={small} title={s.label}>
    {#if s.id === "yt"}<svg viewBox="0 0 24 24" width={small ? 9 : 12} height={small ? 9 : 12}><path d="M8 5.5v13l11-6.5z" fill="currentColor" /></svg>
    {:else if s.id === "ig"}<svg viewBox="0 0 24 24" width={small ? 10 : 13} height={small ? 10 : 13} fill="none" stroke="currentColor" stroke-width="2.4"><rect x="3.5" y="3.5" width="17" height="17" rx="5" /><circle cx="12" cy="12" r="4" /></svg>
    {:else if s.id === "sp"}<svg viewBox="0 0 24 24" width={small ? 10 : 13} height={small ? 10 : 13} fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"><path d="M5 9c4.5-1.5 9.5-1 14 1.5M6 13c3.6-1 7.4-.6 11 1.2M7 16.8c2.8-.7 5.5-.4 8 .9" /></svg>
    {:else if s.id === "clip"}<Icon name="scissors" size={small ? 10 : 13} />
    {:else if s.id === "comp"}<Icon name="layers" size={small ? 10 : 13} />
    {:else if s.id === "web"}<Icon name="link" size={small ? 10 : 13} />
    {:else}<b>{GLYPH[s.id] ?? s.label[0]}</b>{/if}
  </span>
{/snippet}

{#snippet tags(d)}
  {#if d.media}<span class="kb fmt">{d.media_label}</span>{#if d.section_label}<span class="kb cutb" title="Extrait seulement"><Icon name="scissors" size={10} />{d.section_label}</span>{/if}{:else}<span class="kb k-{d.kind}">{KIND_LABEL[d.kind] ?? "Autre"}</span>{/if}
  {#if d.platform}<ConsoleBadge platform={d.platform} small />{/if}
{/snippet}

{#snippet meta(d, pct)}
  <div class="meta">
    {#if d.st !== "done" && d.size > 0}<b>{pct.toFixed(1).replace(".", ",")} %</b><span>{fmtSize(d.done)} / {fmtSize(d.size)}</span>{/if}
    {#if d.st === "done"}<span>{fmtSize(d.size)}</span>{/if}
    {#if d.st === "running" && !d.retrying && d.speed > 0}
      <span class="spd">↓ {fmtSpeed(d.speed)}</span>
      {#if d.size > 0 && fmtEta(d.eta)}<span>fin dans {fmtEta(d.eta)}</span>{/if}
      {#if d.conns > 1}<span class="conns" title="Connexions en parallèle">{d.conns} connexions</span>{/if}
    {/if}
    {#if statusText(d)}<span class="stxt">{statusText(d)}</span>{/if}
    {#if d.st !== "done" && d.size > 0 && !d.resumable}<span class="nores" title="Ce serveur ne permet pas de reprendre : une coupure fait repartir de zéro">sans reprise</span>{/if}
  </div>
{/snippet}

{#snippet acts(d)}
  {#if d.st === "running" || d.st === "queued"}
    <button class="ib" title="Pause" onclick={() => act([d], "pause")}><Icon name="pause" size={15} /></button>
  {:else if d.st === "paused"}
    <button class="ib hot" title="Reprendre" onclick={() => act([d], "resume")}><Icon name="play" size={15} /></button>
  {:else if d.st === "error"}
    <button class="ib" title="Réessayer" onclick={() => act([d], "resume")}><Icon name="refresh" size={15} /></button>
  {:else if romOf(d)}
    {#if isArchive(d) && !d.rom}
      <button class="tb" disabled={extracting === d.id} title="Décompresser l'archive à côté d'elle : le jeu est prêt à lancer" onclick={() => extract(d)}>
        {#if extracting === d.id}<span class="spin"></span>Extraction…{:else}<Icon name="layers" size={14} />Extraire{/if}
      </button>
    {/if}
    <button class="tb play" title="Jouer avec {emuById(romOf(d).emu)?.name ?? 'l\'émulateur'}" onclick={() => play(romOf(d))}><Icon name="play" size={14} />Jouer</button>
  {:else}
    <button class="ib" title="Ouvrir" onclick={() => openItem(d)}><Icon name="open" size={15} /></button>
  {/if}
  <button class="ib" title="Ouvrir le dossier" onclick={() => reveal(d.path, d.dir).catch((e) => flash(String(e)))}><Icon name="folder" size={15} /></button>
  <button class="ib" title="Plus d'actions" onclick={(e) => itemMenu(e, d)}><Icon name="more" size={15} /></button>
{/snippet}

<svelte:window onkeydown={onkey} onpointerdown={bandDown} onpointermove={bandMove} onpointerup={bandUp} onpointercancel={bandUp} />

{#if band?.moved}
  <div class="band" style="left:{Math.min(band.x0, band.x1)}px;top:{Math.min(band.y0, band.y1)}px;width:{Math.abs(band.x1 - band.x0)}px;height:{Math.abs(band.y1 - band.y0)}px"></div>
{/if}

{#if selItems.length}
  <div class="selbar" transition:fly={{ y: 20, duration: 160 }}>
    <span class="count"><b>{selItems.length}</b>sélectionné{selItems.length > 1 ? "s" : ""}</span>
    {#if selItems.some(isBusy)}<button onclick={() => act(selItems.filter(isBusy), "pause")}><Icon name="pause" size={14} />Pause</button>{/if}
    {#if selItems.some((d) => d.st === "paused")}<button onclick={() => act(selItems.filter((d) => d.st === "paused"), "resume")}><Icon name="play" size={14} />Reprendre</button>{/if}
    {#if selItems.some((d) => d.st === "error")}<button onclick={() => act(selItems.filter((d) => d.st === "error"), "resume")}><Icon name="refresh" size={14} />Réessayer</button>{/if}
    <button onclick={() => moveTo(selItems)}><Icon name="move" size={14} />Déplacer</button>
    <button onclick={() => copyLinks(selItems)}><Icon name="copy" size={14} />Copier les liens</button>
    <button class="danger" title="Retirer de la liste (Suppr) · les fichiers terminés restent" onclick={() => remove(selItems, false)}><Icon name="close" size={14} />Retirer</button>
    {#if selItems.some((d) => d.done > 0)}<button class="danger" title="Supprimer avec les fichiers (Maj+Suppr)" onclick={() => remove(selItems, true)}><Icon name="trash" size={14} />Supprimer</button>{/if}
    <button class="x" title="Désélectionner (Échap)" onclick={() => (sel = new Set())}><Icon name="close" size={14} /></button>
  </div>
{/if}

<style>
  .dls { display: flex; flex-direction: column; gap: 12px; padding: 4px 4px 24px; max-width: 1100px; margin: 0 auto; width: 100%; }
  .add { display: flex; gap: 8px; }
  .field { flex: 1; display: flex; align-items: center; gap: 10px; height: 40px; padding: 0 12px; border-radius: 10px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--muted); }
  .field:focus-within { border-color: var(--accent); box-shadow: 0 0 0 3px var(--ring); }
  .field input { flex: 1; border: 0; outline: none; background: none; color: var(--text); font: inherit; font-size: 13.5px; }
  .bar { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .spacer { flex: 1; }
  .seg { display: inline-flex; padding: 3px; border-radius: 10px; background: var(--chip); }
  .seg button { display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 11px; border: 0; border-radius: 7px; background: none; color: var(--muted); font: inherit; font-size: 12.5px; font-weight: 600; cursor: pointer; }
  .seg button.on { background: var(--surface); color: var(--text); box-shadow: var(--shadow-sm); }
  .seg button.warn .n { background: var(--err); color: white; }
  .seg .n { font-size: 10.5px; padding: 0 5px; border-radius: 6px; background: var(--line); color: var(--text-2); }
  .speed { display: inline-flex; align-items: center; gap: 4px; font-size: 12.5px; font-weight: 700; color: var(--accent); font-variant-numeric: tabular-nums; }
  .sbtn { display: inline-flex; align-items: center; gap: 5px; height: 28px; padding: 0 10px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text-2); font: inherit; font-size: 12px; font-weight: 600; cursor: pointer; }
  .sbtn:hover { color: var(--text); border-color: var(--accent); }
  .sbtn.hot, .ib.hot { color: #b45309; border-color: rgb(245 158 11 / 0.45); background: rgb(245 158 11 / 0.12); }

  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  li { display: flex; align-items: center; gap: 12px; padding: 10px; border-radius: 12px; background: var(--surface); border: 1px solid var(--line); cursor: pointer; user-select: none; transition: border-color 0.12s, box-shadow 0.12s, transform 0.12s, background 0.12s; }
  li:hover { border-color: var(--accent); background: var(--bg-2); box-shadow: 0 4px 14px rgb(0 0 0 / 0.08), 0 0 0 1px var(--accent); transform: translateY(-1px); }
  li:active { transform: translateY(0); }
  li:hover .title { color: var(--accent); }
  li.st-error { border-color: rgb(225 29 72 / 0.35); }
  .thumb { flex: none; width: 42px; height: 60px; border-radius: 7px; overflow: hidden; display: grid; place-items: center; background: var(--chip); color: var(--muted); }
  .thumb img { width: 100%; height: 100%; object-fit: cover; }
  .thumb.wide { width: 96px; height: 54px; }
  .kb.fmt { background: var(--accent-soft); color: var(--accent); }
  .fmts { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; margin-top: -4px; }
  .fl { font-size: 12px; font-weight: 650; color: var(--muted); margin-right: 2px; }
  .fmts button { display: inline-flex; align-items: center; gap: 5px; height: 30px; padding: 0 11px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text-2); font: inherit; font-size: 12.5px; font-weight: 600; cursor: pointer; }
  .fmts button:hover { color: var(--text); border-color: var(--accent); }
  .fmts button.on { background: var(--accent); border-color: var(--accent); color: white; }
  .fmts .vsep { width: 1px; height: 18px; background: var(--line-strong); margin: 0 4px; }
  .cut { display: inline-flex; align-items: center; gap: 6px; font-size: 12.5px; color: var(--muted); }
  .cut input { width: 70px; height: 30px; padding: 0 8px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: inherit; font-size: 12.5px; font-variant-numeric: tabular-nums; text-align: center; }
  .cut input:focus { outline: none; border-color: var(--accent); box-shadow: 0 0 0 3px var(--ring); }
  .cut.bad input { border-color: var(--err); }
  .kb.cutb { display: inline-flex; align-items: center; gap: 3px; font-variant-numeric: tabular-nums; }
  .pv { display: flex; align-items: center; gap: 12px; margin-top: -4px; padding: 8px; border-radius: 12px; background: var(--surface); border: 1px solid var(--line); min-height: 40px; }
  .pv img { width: 112px; height: 63px; object-fit: cover; border-radius: 8px; flex: none; }
  .pvt { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .pvt b { font-size: 14px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .pvt span { font-size: 12.5px; color: var(--muted); font-variant-numeric: tabular-nums; }
  .pverr { display: inline-flex; align-items: center; gap: 6px; font-size: 12.5px; color: var(--err); padding: 0 6px; }
  .pvload { display: inline-flex; align-items: center; gap: 8px; font-size: 12.5px; color: var(--muted); padding: 0 6px; }
  .spin { width: 14px; height: 14px; border-radius: 50%; border: 2px solid var(--line-strong); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .fmts .xcut { width: 30px; padding: 0; justify-content: center; }
  .sp-note { display: flex; align-items: center; gap: 6px; margin: -4px 0 0; font-size: 12.5px; color: var(--muted); }
  .main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .top { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .title { font-size: 14px; font-weight: 650; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .kb { flex: none; font-size: 10.5px; font-weight: 700; padding: 1px 6px; border-radius: 5px; background: var(--chip); color: var(--text-2); }
  .fname { font-size: 11.5px; color: var(--muted); font-family: ui-monospace, "Cascadia Code", monospace; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .fname.site { font-family: inherit; font-size: 12px; }
  .pbar { height: 5px; border-radius: 5px; background: var(--chip); overflow: hidden; margin-top: 2px; }
  .pbar span { display: block; height: 100%; border-radius: 5px; background: var(--accent); transition: width 0.8s ease; }
  .st-paused .pbar span { background: #f59e0b; }
  .retry .pbar span, .st-queued .pbar span { background: var(--muted); }
  .st-error .pbar span { background: var(--err); }
  .meta { display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px 12px; font-size: 12px; color: var(--muted); font-variant-numeric: tabular-nums; }
  .meta b { color: var(--text); }
  .spd { color: var(--accent); font-weight: 700; }
  .conns { font-size: 11px; padding: 0 6px; border-radius: 5px; background: var(--accent-soft); color: var(--accent); font-weight: 650; }
  .st-error .stxt { color: var(--err); font-weight: 600; }
  .retry .stxt, .st-paused .stxt { color: #d97706; font-weight: 600; }
  .st-done .stxt { color: var(--ok); }
  .nores { font-size: 11px; color: var(--muted); border: 1px dashed var(--line-strong); border-radius: 5px; padding: 0 5px; }
  .acts { display: flex; align-items: center; gap: 2px; flex: none; }
  .tb { display: inline-flex; align-items: center; gap: 6px; height: 32px; padding: 0 12px; margin-right: 4px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text-2); font: inherit; font-size: 12.5px; font-weight: 650; cursor: pointer; }
  .tb:hover:not(:disabled) { border-color: var(--accent); color: var(--accent); }
  .tb.play { background: #16a34a; border-color: #16a34a; color: white; }
  .tb.play:hover { background: #15803d; color: white; }
  li.game .thumb { width: 50px; height: 70px; box-shadow: 0 3px 10px rgb(0 0 0 / 0.18); }
  .fname.rom { display: inline-flex; align-items: center; gap: 5px; color: #16a34a; font-weight: 600; }
  .kb.k-game { background: rgb(22 163 74 / 0.12); color: #16a34a; }
  .ib { display: grid; place-items: center; width: 32px; height: 32px; border-radius: 8px; border: 1px solid transparent; background: transparent; color: var(--text-2); cursor: pointer; }
  .ib:hover { background: var(--chip); color: var(--text); }
  .empty { display: flex; flex-direction: column; align-items: center; gap: 12px; padding: 60px 20px; color: var(--muted); text-align: center; }
  .empty p { margin: 0; max-width: 460px; line-height: 1.55; font-size: 13px; }
  .empty b { color: var(--text); font-size: 15px; }
  .hint { color: var(--muted); font-size: 13px; padding: 20px 6px; margin: 0; }

  /* Bascule liste / cartes */
  .seg.vw button { padding: 0 9px; }

  /* Sélection */
  li.sel, .dc.sel { border-color: var(--accent); background: var(--accent-soft); box-shadow: 0 0 0 1.5px var(--accent); }
  li.sel:hover { background: var(--accent-soft); }
  .thumb { position: relative; }
  .thumb .ico { display: grid; place-items: center; }
  .corner { position: absolute; left: 3px; bottom: 3px; }

  /* Logo du site d'origine */
  .site { display: grid; place-items: center; width: 22px; height: 22px; border-radius: 6px; background: #475569; color: white; box-shadow: 0 1px 4px rgb(0 0 0 / 0.3); font-size: 12px; line-height: 1; }
  .site.sm { width: 16px; height: 16px; border-radius: 4px; font-size: 9.5px; }
  .site b { font-weight: 800; font-family: system-ui, sans-serif; }
  .s-yt { background: #ff0033; }
  .s-x, .s-tt, .s-th, .s-kk { background: #111; }
  .s-kk { color: #53fc18; }
  .s-ig { background: radial-gradient(circle at 30% 110%, #fdc468 0%, #fa6d3e 30%, #d62976 60%, #962fbf 85%, #4f5bd5 100%); }
  .s-fb { background: #1877f2; }
  .s-vm { background: #1ab7ea; }
  .s-dm { background: #0d0d0d; }
  .s-tw { background: #9146ff; }
  .s-rd { background: #ff4500; }
  .s-sc { background: #ff5500; }
  .s-sp { background: #1db954; color: #111; }
  .s-bl { background: #00a1d6; }
  .s-bs { background: #1185fe; }
  .s-pi { background: #e60023; }
  .s-bc { background: #1da0c3; font-size: 10px; }
  .s-st { background: #0f90fa; }
  .s-sn { background: #fffc00; color: #111; }
  .s-clip, .s-comp { background: var(--accent); }
  .s-file { background: rgb(0 0 0 / 0.55); backdrop-filter: blur(4px); }

  /* Vue cartes */
  .cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(230px, 1fr)); gap: 12px; user-select: none; }
  .dc { position: relative; display: flex; flex-direction: column; border-radius: 14px; background: var(--surface); border: 1px solid var(--line); overflow: hidden; cursor: pointer; transition: border-color 0.12s, box-shadow 0.12s, transform 0.12s; }
  .dc:hover { border-color: var(--accent); box-shadow: 0 8px 22px rgb(0 0 0 / 0.1), 0 0 0 1px var(--accent); transform: translateY(-2px); }
  .dc:hover .title { color: var(--accent); }
  .dc.st-error { border-color: rgb(225 29 72 / 0.35); }
  .cv { position: relative; aspect-ratio: 16 / 9; display: grid; place-items: center; overflow: hidden; background: var(--chip); color: var(--muted); }
  .cv > img { position: relative; width: 100%; height: 100%; object-fit: cover; }
  .cv.poster > img:not(.blur) { object-fit: contain; filter: drop-shadow(0 4px 12px rgb(0 0 0 / 0.35)); padding: 8px 0; }
  .cv > img.blur { position: absolute; inset: 0; object-fit: cover; filter: blur(18px) saturate(1.3); transform: scale(1.2); opacity: 0.75; }
  .cv .ico { display: grid; place-items: center; opacity: 0.7; }
  .cv-tl { position: absolute; top: 8px; left: 8px; display: flex; gap: 4px; }
  .cv-tr { position: absolute; top: 8px; right: 8px; display: flex; gap: 4px; align-items: center; }
  .pill { display: inline-flex; align-items: center; gap: 3px; height: 20px; padding: 0 7px; border-radius: 6px; background: rgb(0 0 0 / 0.6); backdrop-filter: blur(4px); color: white; font-size: 10.5px; font-weight: 700; font-variant-numeric: tabular-nums; }
  .tick { position: absolute; top: 8px; right: 8px; display: grid; place-items: center; width: 24px; height: 24px; border-radius: 50%; background: var(--accent); color: white; box-shadow: 0 2px 8px rgb(0 0 0 / 0.35); }
  .dc.sel .cv-tr { opacity: 0; }
  .cv-pct { position: absolute; left: 8px; bottom: 11px; padding: 2px 7px; border-radius: 6px; background: rgb(0 0 0 / 0.6); color: white; font-size: 11px; font-weight: 700; font-variant-numeric: tabular-nums; }
  .cv-bar { position: absolute; left: 0; right: 0; bottom: 0; height: 4px; background: rgb(0 0 0 / 0.3); }
  .cv-bar span { display: block; height: 100%; background: var(--accent); transition: width 0.8s ease; }
  .st-paused .cv-bar span { background: #f59e0b; }
  .retry .cv-bar span, .st-queued .cv-bar span { background: var(--muted); }
  .st-error .cv-bar span { background: var(--err); }
  .dc-body { display: flex; flex-direction: column; gap: 3px; padding: 9px 11px 4px; min-width: 0; }
  .title.t2 { white-space: normal; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; line-height: 1.3; min-height: 2.6em; }
  .dc-body .fname.rom { color: #16a34a; font-weight: 600; }
  .dc-acts { display: flex; align-items: center; justify-content: flex-end; gap: 2px; margin-top: auto; padding: 0 6px 6px; }
  .dc-acts .tb { height: 30px; padding: 0 10px; }
  .dc-acts .tb.play { margin-right: auto; margin-left: 4px; }

  .band { position: fixed; z-index: 30; border: 1.5px solid var(--accent); background: var(--accent-soft); border-radius: 4px; pointer-events: none; }
  .selbar {
    position: fixed; left: 50%; bottom: 22px; translate: -50% 0; z-index: 35; display: flex; align-items: center; gap: 2px; padding: 6px 6px 6px 8px; border-radius: 18px;
    background: linear-gradient(135deg, color-mix(in srgb, var(--accent) 82%, white), var(--accent) 45%, color-mix(in srgb, var(--accent) 80%, black));
    color: white;
    box-shadow: 0 14px 34px -8px color-mix(in srgb, var(--accent) 60%, transparent), 0 2px 6px rgb(0 0 0 / 0.15), inset 0 1px 0 rgb(255 255 255 / 0.25);
  }
  .selbar .count { display: inline-flex; align-items: center; gap: 8px; padding: 0 12px 0 2px; margin-right: 4px; border-right: 1px solid rgb(255 255 255 / 0.25); font-weight: 700; font-size: 13px; white-space: nowrap; }
  .selbar .count b { display: grid; place-items: center; min-width: 26px; height: 26px; padding: 0 7px; border-radius: 13px; background: white; color: var(--accent); font-size: 13px; font-weight: 800; }
  .selbar button { display: inline-flex; align-items: center; gap: 6px; height: 34px; padding: 0 12px; border: 0; border-radius: 11px; background: none; color: white; font: inherit; font-size: 12.5px; font-weight: 600; cursor: pointer; white-space: nowrap; transition: background 0.12s; }
  .selbar button:hover { background: rgb(255 255 255 / 0.18); }
  .selbar button.danger { background: rgb(255 255 255 / 0.12); margin-left: 2px; }
  .selbar button.danger:hover { background: #e11d48; }
  .selbar .x { width: 34px; padding: 0; justify-content: center; margin-left: 4px; background: rgb(0 0 0 / 0.15); border-radius: 50%; }
</style>
