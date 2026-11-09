<script>
  import { fly, fade } from "svelte/transition";
  import Icon from "./Icon.svelte";
  import Files from "./Files.svelte";
  import Sources from "./Sources.svelte";
  import { KINDS, STATUS_LABEL, fmtEta, fmtPos, fmtSize, fmtSpeed, hueOf, isDone, isPaused, partialLabel, subtitle } from "./model.js";
  import { getWatch, setWatched } from "./api.js";
  import { flash, pickArt } from "./ui.svelte.js";
  import { emuFindRoms } from "./api.js";
  import { emuById, emus, loadEmus, play } from "./emu.svelte.js";
  import { arts, schedules, wantSchedule, airTime, fmtAir, releaseGroup, alignEpisodes } from "./ui.svelte.js";

  let { group: g, onclose, onaction, onitemmenu } = $props();

  const single = $derived(g.items.length === 1);
  let openFiles = $state(new Set());
  const kindLabel = $derived(KINDS.find((k) => k.id === g.kind)?.label ?? "");
  const incomplete = $derived(g.items.filter((i) => i.progress < 1 && i.status !== "error"));
  const anyRunning = $derived(incomplete.some((i) => !isPaused(i.status)));
  const anyPaused = $derived(incomplete.some((i) => isPaused(i.status)));
  // Lecture directe : seulement un film (un livre audio contient plein de fichiers → on choisit dans la liste)
  const canPlay = $derived(g.kind === "film" && single && g.items[0].progress > 0);
  const isShow = $derived(g.kind === "series" || g.kind === "anime");

  // ---------- calendrier de diffusion ----------
  $effect(() => wantSchedule(g));
  const sched = $derived(isShow ? schedules[g.artKey] : null);
  let now = $state(Date.now());
  $effect(() => {
    const id = setInterval(() => (now = Date.now()), 30000);
    return () => clearInterval(id);
  });
  const curSeason = $derived(Math.max(0, ...g.items.map((i) => i.season ?? 0)));
  // Épisodes de la saison en cours (AniList : saison 0 = « en cours »)
  const ownedNums = $derived(new Set(g.items.filter((i) => (i.season ?? curSeason) === curSeason && i.episode != null).map((i) => i.episode)));
  const seasonEps = $derived(alignEpisodes(sched, ownedNums, curSeason, now));
  const nextEp = $derived.by(() => {
    if (!sched) return null;
    // Prochain épisode pas encore téléchargé (certaines équipes sortent un épisode en avance)
    const future = seasonEps.filter((e) => airTime(e) > now && !ownedNums.has(e.number)).sort((a, b) => airTime(a) - airTime(b));
    return future[0] ?? null;
  });
  // Liste : épisodes téléchargés + épisodes sortis manquants + prochain épisode (en pointillés)
  const rows = $derived.by(() => {
    const r = g.items.map((it) => ({ key: it.hash, type: "item", it, s: it.season ?? 0, n: it.episode ?? 9999 }));
    if (!isShow || !sched || !ownedNums.size) return r;
    const minOwned = Math.min(...ownedNums);
    for (const ep of seasonEps) {
      if (ownedNums.has(ep.number) || ep.number < minOwned) continue;
      const t = airTime(ep);
      if (t && t <= now) r.push({ key: `m${ep.number}`, type: "missing", ep, s: curSeason, n: ep.number });
    }
    if (nextEp && !ownedNums.has(nextEp.number)) r.push({ key: `n${nextEp.number}`, type: "next", ep: nextEp, s: curSeason, n: nextEp.number });
    return r.sort((a, b) => a.s - b.s || a.n - b.n);
  });
  const missingCount = $derived(rows.filter((r) => r.type === "missing").length);

  /** Recherche préremplie pour un épisode : même titre, même format de numéro, même équipe mise en avant. */
  function searchEp(n) {
    const latest = g.items[g.items.length - 1];
    const pad = (x) => String(x).padStart(2, "0");
    const usesSE = /S\d+E\d+/i.test(latest.name);
    const code = usesSE ? `S${pad(curSeason || 1)}E${pad(n)}` : pad(n);
    const source = latest.name.trim().startsWith("[") ? "nyaa" : g.kind === "anime" && !/MULTI|VFF|VOSTFR|FRENCH/i.test(latest.name) ? "nyaa" : "c411";
    onaction("searchEpisode", { query: `${latest.title} ${code}`, source, prefer: releaseGroup(latest.name), code });
  }
  const isVideo = $derived(g.kind === "film" || isShow);

  // ---------- vu / en cours (historique de VLC), pour les séries en plusieurs torrents ----------
  let watch = $state({});
  async function loadWatch() {
    if (!isVideo || single) return;
    try {
      watch = await getWatch(g.items.map((i) => i.content_path).filter(Boolean));
    } catch {}
  }
  $effect(() => {
    g.key;
    loadWatch();
    const id = setInterval(() => !document.hidden && loadWatch(), 4000);
    return () => clearInterval(id);
  });
  const w = (it) => watch[it.content_path];
  // Épisodes téléchargés, dans l'ordre de la liste
  const epItems = $derived(rows.filter((r) => r.type === "item").map((r) => r.it));
  const resume = $derived.by(() => {
    const started = epItems.filter((i) => w(i)?.pos_ms > 0 && !w(i)?.watched).sort((a, b) => w(b).at - w(a).at)[0];
    if (started) return { it: started, pos: w(started).pos_ms };
    let last = -1;
    epItems.forEach((i, n) => w(i)?.watched && (last = n));
    if (last < 0) return null;
    const next = epItems.slice(last + 1).find((i) => !w(i)?.watched && i.progress > 0);
    return next ? { it: next, pos: 0 } : null;
  });
  const seenCount = $derived(epItems.filter((i) => w(i)?.watched).length);
  async function markSeen(list, seen) {
    try {
      await setWatched(list.map((i) => i.content_path), seen);
      loadWatch();
    } catch (e) {
      flash(String(e));
    }
  }
  /** Entrées « vu » ajoutées au clic droit d'un épisode */
  const watchMenu = (it) =>
    !isVideo || single
      ? []
      : [
          w(it)?.watched
            ? { label: "Marquer comme pas vu", icon: "eye", run: () => markSeen([it], false) }
            : { label: "Marquer comme vu", icon: "eye", run: () => markSeen([it], true) },
          { label: "Marquer vus jusqu'ici", icon: "check", run: () => markSeen(epItems.slice(0, epItems.findIndex((x) => x.hash === it.hash) + 1), true) },
        ];
  // Groupe : dossier commun ; torrent seul : son contenu
  const location = $derived(single ? g.items[0].content_path || g.items[0].save_path : g.items[0].save_path);

  function epLabel(it) {
    if (it.season != null && it.episode != null) return `S${it.season} · É${it.episode}`;
    if (it.episode != null) return `É${it.episode}`;
    if (it.season != null) return `Saison ${it.season}`;
    return null;
  }
  function prettyName(n) {
    const s = n.replace(/\.[a-z0-9]{2,4}$/i, "");
    return (s.match(/[._]/g) ?? []).length > (s.match(/ /g) ?? []).length ? s.replace(/[._]/g, " ") : s;
  }
  // Installation Windows : seulement pour les jeux PC / logiciels (pas un .nsp de Switch…)
  const installable = $derived((g.kind === "game" || g.kind === "software") && !g.items.some((i) => i.platform));
  let tab = $state("files");

  // Jeu console téléchargé : ses ROM, à lancer dans le bon émulateur
  const isConsole = $derived(g.items.some((i) => i.platform));
  const romsReady = $derived(isConsole && g.items.every((i) => i.progress >= 1));
  let roms = $state([]);
  let romSel = $state(0);
  $effect(() => {
    if (!romsReady) {
      roms = [];
      return;
    }
    const loc = location;
    if (!emus.list.length) loadEmus();
    emuFindRoms(loc).then((r) => {
      if (loc === location) (roms = r), (romSel = 0);
    });
  });
  const rom = $derived(roms[romSel]);
  const romEmu = $derived(rom?.emu ? emuById(rom.emu) : null);
  const installed = $derived(g.items.find((i) => i.installed)?.installed);
  const art = $derived(arts[g.artKey]);
  let posterOk = $state(true);
  $effect(() => {
    art;
    posterOk = true;
  });

  function label(it) {
    if (it.season != null && it.episode != null) return `S${String(it.season).padStart(2, "0")}E${String(it.episode).padStart(2, "0")}`;
    if (it.season != null) return `Saison ${it.season}`;
    return null;
  }
  function toggleFiles(h) {
    const s = new Set(openFiles);
    s.has(h) ? s.delete(h) : s.add(h);
    openFiles = s;
  }
</script>

<div class="backdrop" transition:fade={{ duration: 150 }} onclick={onclose} role="presentation"></div>

<aside class="drawer" style="--h:{hueOf(g.artKey)}" transition:fly={{ x: 40, duration: 220 }}>
  <header class="hero">
    {#if art && posterOk}
      <img class="blur" src={art.url} alt="" />
    {/if}
    <div class="hero-actions">
      <button class="icon-btn close" title="Fermer (Échap)" onclick={onclose}><Icon name="close" size={15} /></button>
    </div>
    <div class="hero-row">
      <button class="poster-btn" title="Choisir l'image" onclick={() => pickArt(g)}>
        {#if art && posterOk}
          <img class="poster" src={art.url} alt="" onerror={() => (posterOk = false)} />
        {:else}
          <span class="poster ph"><Icon name={g.kind} size={30} /></span>
        {/if}
        <span class="poster-edit"><Icon name="image" size={16} /></span>
      </button>
      <div class="hero-text">
        <span class="kind"><Icon name={g.kind} size={12} />{kindLabel}</span>
        <h2>{g.title}</h2>
        <p>{[subtitle(g), g.partial ? `${fmtSize(g.size)} sur ${fmtSize(g.totalSize)} · ${partialLabel(g)}` : fmtSize(g.size)].filter(Boolean).join(" · ")}</p>
      </div>
      <!-- Pause / reprendre / lire : centré verticalement sur l'affiche -->
      <div class="hero-main">
        {#if anyRunning}
          <button class="big" title={single ? "Mettre en pause" : "Tout mettre en pause"} onclick={() => onaction("pause", incomplete)}><Icon name="pause" size={20} /></button>
        {:else if anyPaused}
          <button class="big hot" title={single ? "Reprendre" : "Tout reprendre"} onclick={() => onaction("resume", incomplete)}><Icon name="play" size={20} /></button>
        {:else if canPlay}
          <button class="big play" title="Lire" onclick={() => onaction("play", g.items)}><Icon name="play" size={20} /></button>
        {/if}
      </div>
    </div>
    {#if g.progress < 1 && g.status !== "error"}
      <!-- Progression globale, toujours visible en haut -->
      <div class="hero-progress" class:paused={anyPaused && !anyRunning}>
        <div class="hp-bar"><span style="width:{g.progress * 100}%"></span></div>
        <div class="hp-text">
          <b>{(g.progress * 100).toFixed(1).replace(".", ",")} %</b>
          {#if anyPaused && !anyRunning}
            <span>En pause</span>
          {:else if g.dlspeed > 0}
            <span>↓ {fmtSpeed(g.dlspeed)}</span>
            {#if fmtEta(g.eta)}<span>fin dans {fmtEta(g.eta)}</span>{/if}
          {:else}
            <span>{STATUS_LABEL[g.status]}</span>
          {/if}
        </div>
      </div>
    {/if}
  </header>

  <!-- Emplacement + actions en icônes, sur une seule ligne -->
  <div class="pathbar">
    <span class="path" title={location}>{location}</span>
    <button class="ib" title="Ouvrir le dossier" onclick={() => onaction("reveal", { content_path: location, save_path: g.items[0].save_path })}><Icon name="folder" size={15} /></button>
    <button class="ib" title="Déplacer…" onclick={() => onaction("move", g.items)}><Icon name="move" size={15} /></button>
    <button class="ib" title="Partager le .torrent" onclick={() => onaction("share", g.items)}><Icon name="share" size={15} /></button>
    <button class="ib danger" title={single ? "Supprimer" : `Supprimer les ${g.items.length} torrents`} onclick={(e) => onaction("removeMenu", { e, items: g.items })}><Icon name="trash" size={15} /></button>
    <button class="ib" title="Plus d'actions" onclick={(e) => onitemmenu(e, g.items[0], g)}><Icon name="more" size={15} /></button>
  </div>

  {#if rom}
    <div class="install ok play-band">
      <Icon name="game" size={16} />
      <div class="it">
        <b>{romEmu ? `Jouer avec ${romEmu.name}` : "Jouer"}</b>
        {#if roms.length > 1}
          <select bind:value={romSel} title="Quel fichier lancer">
            {#each roms as r, i}<option value={i}>{r.name}</option>{/each}
          </select>
        {:else}
          <span title={rom.path}>{romEmu ? (romEmu.installed ? romEmu.consoles : `${romEmu.consoles} · s'installe au premier lancement`) : rom.name}</span>
        {/if}
      </div>
      <button class="sbtn primary" onclick={() => play(rom)}><Icon name="play" size={12} />Jouer</button>
    </div>
  {/if}

  {#if installable}
    {#if installed}
      <div class="install ok">
        <Icon name="check" size={16} />
        <div class="it"><b>Installé</b><span title={installed.location}>{installed.name}</span></div>
        {#if installed.location}<button class="ib" title="Ouvrir le dossier du jeu / logiciel" onclick={() => onaction("openInstall", installed)}><Icon name="folder" size={15} /></button>{/if}
        <button class="sbtn" onclick={() => onaction("uninstall", installed)}>Désinstaller</button>
        <button class="sbtn danger" onclick={() => onaction("deleteInstaller", g.items)}><Icon name="trash" size={13} />Supprimer l'installable</button>
      </div>
    {:else}
      <div class="install">
        <Icon name="alert" size={16} />
        <div class="it"><b>Pas installé</b><span>Aucun programme correspondant trouvé dans Windows</span></div>
        {#if single && g.items[0].progress >= 1}<button class="sbtn primary" onclick={() => onaction("setup", g.items[0])}>Installer</button>{/if}
      </div>
    {/if}
  {/if}

  {#if isShow && sched && (nextEp || missingCount)}
    <!-- Diffusion : prochain épisode et épisodes sortis à télécharger -->
    <div class="airing">
      <Icon name="calendar" size={16} />
      {#if nextEp}
        <span class="ai-next">
          <b>Prochain : É{nextEp.number}</b>{#if nextEp.name} « {nextEp.name} »{/if}
          <em>{fmtAir(airTime(nextEp), now)} · {new Date(airTime(nextEp)).toLocaleString("fr-FR", { weekday: "short", hour: "2-digit", minute: "2-digit" })}</em>
        </span>
      {/if}
      {#if missingCount}
        <button class="ai-miss" onclick={() => (tab = "files")}>{missingCount} épisode{missingCount > 1 ? "s" : ""} sorti{missingCount > 1 ? "s" : ""} à récupérer</button>
      {/if}
    </div>
  {/if}

  <!-- Onglets : contenu / sources -->
  <div class="tabs">
    <button class:on={tab === "files"} onclick={() => (tab = "files")}>
      <Icon name="list" size={14} />{single ? "Fichiers" : isShow ? "Épisodes" : "Éléments"}
    </button>
    <button class:on={tab === "sources"} onclick={() => (tab = "sources")}>
      <Icon name="up" size={14} />Sources
      <span class="tn" class:bad={incomplete.length && g.items.every((i) => i.seeds_total === 0)}>{g.items.reduce((s, i) => s + i.seeds, 0)}</span>
    </button>
  </div>

  {#if tab === "sources"}
    <div class="pad"><Sources items={g.items} /></div>
  {:else if single}
    {@const it = g.items[0]}
    {#if it.status === "error"}
      <div class="warn"><Icon name="alert" size={15} />Fichiers introuvables (déplacés ou supprimés).
        <button class="sbtn" onclick={() => onaction("recheck", [it])}><Icon name="refresh" size={13} />Revérifier</button>
      </div>
    {/if}
    <div class="pad"><Files torrent={it} {onaction} fill episodes={g.kind === "series" || g.kind === "anime"} /></div>
  {:else}
    <!-- Plusieurs torrents (un par épisode) : affichés comme UN seul torrent avec sa liste d'épisodes -->
    {@const doneCount = g.items.filter((i) => i.progress >= 1).length}
    <div class="pad">
      <div class="eps-head">
        <b>{doneCount}</b><span>/{g.items.length} {isShow ? "épisodes" : "éléments"} téléchargés</span>
        <em>{fmtSize(g.size)}</em>
        {#if resume}
          <button class="resume" title="Lancer dans VLC (il reprend là où tu t'es arrêté)" onclick={() => onaction("play", [resume.it])}>
            <Icon name="play" size={11} />{resume.pos ? "Reprendre" : "Suivant"} : {epLabel(resume.it) ?? resume.it.title}{#if resume.pos}<i>{fmtPos(resume.pos)}</i>{/if}
          </button>
        {:else if seenCount}
          <span class="seen-n"><Icon name="eye" size={12} />{seenCount} vu{seenCount > 1 ? "s" : ""}</span>
        {/if}
      </div>
      <ul class="eps">
        {#each rows as row (row.key)}
          {#if row.type !== "item"}
            <!-- Épisode pas encore téléchargé : sorti (à chercher) ou à venir -->
            {@const t = airTime(row.ep)}
            <li class="ghost" class:upcoming={row.type === "next"}>
              <span class="ep">{curSeason ? `S${curSeason} · ` : ""}É{row.ep.number}</span>
              <div class="body">
                <span class="name">{row.ep.name || `Épisode ${row.ep.number}`}</span>
              </div>
              <span class="meta"><em class:soon={row.type === "next"}>{fmtAir(t, now)}</em></span>
              {#if row.type === "missing"}
                <button class="sbtn primary" onclick={() => searchEp(row.ep.number)}><Icon name="search" size={13} />Chercher</button>
              {:else}
                <button class="sbtn" disabled title="Pas encore sorti"><Icon name="search" size={13} />Bientôt</button>
              {/if}
            </li>
          {:else}
          {@const it = row.it}
          {@const pct = Math.floor(it.progress * 1000) / 10}
          {@const done = it.progress >= 1}
          {@const paused = isPaused(it.status) && !done}
          {@const wi = w(it)}
          <li
            class:done
            class:paused
            class:err={it.status === "error"}
            class:seen={wi?.watched}
            oncontextmenu={(e) => onitemmenu(e, it, null, watchMenu(it), isVideo && !single)}
            ondblclick={(e) => !e.target.closest("button") && it.progress > 0 && onaction(isVideo ? "play" : "reveal", isVideo ? [it] : it)}
          >
            {#if epLabel(it)}<span class="ep">{epLabel(it)}</span>{/if}
            <div class="body">
              <span class="name" title={it.name}>{prettyName(it.name)}</span>
              {#if !done}<div class="bar"><span style="width:{pct}%"></span></div>{/if}
            </div>
            <span class="meta">
              {#if wi?.watched}
                <span class="wbadge seen" title="Vu (d'après VLC)"><Icon name="eye" size={11} />Vu</span>
              {:else if wi?.pos_ms > 0}
                <span class="wbadge mid" title="Arrêté à {fmtPos(wi.pos_ms)} dans VLC"><Icon name="pause" size={9} />{fmtPos(wi.pos_ms)}</span>
              {/if}
              {#if it.status === "error"}
                <span class="bad">Manquant</span>
              {:else if done}
                <span class="ok"><Icon name="check" size={12} /></span>{fmtSize(it.size)}
              {:else if paused}
                <span class="pz">En pause</span>{pct}%
              {:else}
                {#if it.dlspeed > 0 && fmtEta(it.eta)}<em>≈ {fmtEta(it.eta)}</em>{/if}{pct}% · {fmtSize(it.size)}
              {/if}
            </span>
            {#if paused}
              <button class="mini" title="Reprendre" onclick={() => onaction("resume", [it])}><Icon name="play" size={13} /></button>
            {:else if !done && it.status !== "error"}
              <button class="mini" title="Pause" onclick={() => onaction("pause", [it])}><Icon name="pause" size={13} /></button>
            {/if}
            {#if isVideo}
              <button class="mini play" title="Lire" disabled={it.progress <= 0 || it.status === "error"} onclick={() => onaction("play", [it])}><Icon name="play" size={11} /></button>
            {/if}
            <button class="mini" title="Plus d'actions" onclick={(e) => onitemmenu(e, it, null, watchMenu(it), isVideo && !single)}><Icon name="more" size={14} /></button>
          </li>
          {/if}
        {/each}
      </ul>
      <p class="tip">Double-clic : lire · clic droit : actions sur l'épisode</p>
    </div>
  {/if}
</aside>

<style>
  .backdrop { position: fixed; inset: 0; background: var(--scrim); z-index: 40; }
  .drawer {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    width: min(580px, 94vw);
    background: var(--bg-2);
    border-left: 1px solid var(--line);
    z-index: 50;
    box-shadow: var(--shadow-lg);
    /* La liste des fichiers / épisodes occupe toute la hauteur restante */
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .drawer > :global(*) { flex: none; }
  .hero {
    position: relative;
    overflow: hidden;
    padding: 16px 18px;
    color: white;
    background: linear-gradient(135deg, hsl(var(--h) 60% 26%), hsl(calc(var(--h) + 60) 55% 12%));
  }
  .blur { position: absolute; inset: -40px; width: calc(100% + 80px); height: calc(100% + 80px); object-fit: cover; filter: blur(36px) brightness(0.5) saturate(1.3); }
  .hero-actions { position: absolute; top: 12px; right: 12px; z-index: 2; }
  .hero-main { margin-left: auto; flex: none; display: flex; align-items: center; }
  .close { width: 30px; height: 30px; background: rgb(8 10 16 / 0.5); color: white; border-color: rgb(255 255 255 / 0.15); }
  .close:hover { color: white; border-color: white; }
  .big { display: grid; place-items: center; width: 50px; height: 50px; border-radius: 50%; border: 0; background: white; color: #111; cursor: pointer; box-shadow: 0 8px 24px rgb(0 0 0 / 0.4); transition: scale 0.12s; }
  .big:hover { scale: 1.07; }
  .big.hot { background: #f59e0b; color: #1a1205; }
  .big.play { background: var(--accent); color: white; padding-left: 3px; }
  .hero-row { position: relative; display: flex; gap: 14px; align-items: center; padding-right: 34px; }
  .hero-progress { position: relative; margin-top: 14px; display: flex; flex-direction: column; gap: 6px; }
  .hp-bar { height: 6px; border-radius: 6px; background: rgb(255 255 255 / 0.18); overflow: hidden; }
  .hp-bar span { display: block; height: 100%; border-radius: 6px; background: linear-gradient(90deg, #2b88ff, #4cc2ff); box-shadow: 0 0 12px #4aa8ff; transition: width 0.6s ease; }
  .hero-progress.paused .hp-bar span { background: #f59e0b; box-shadow: none; }
  .hp-text { display: flex; gap: 12px; align-items: baseline; font-size: 12.5px; color: rgb(255 255 255 / 0.8); }
  .hp-text b { font-size: 15px; color: white; font-variant-numeric: tabular-nums; }
  .hero-progress.paused .hp-text span:first-of-type { color: #fbbf24; font-weight: 700; }
  .poster { width: 64px; aspect-ratio: 2 / 3; object-fit: cover; border-radius: 8px; box-shadow: 0 8px 20px rgb(0 0 0 / 0.5); flex: none; }
  .poster-btn { position: relative; flex: none; padding: 0; border: 0; background: none; border-radius: 8px; cursor: pointer; }
  .poster-btn .poster { display: block; }
  .poster-edit { position: absolute; inset: 0; display: grid; place-items: center; border-radius: 8px; background: rgb(0 0 0 / 0.5); color: white; opacity: 0; transition: opacity 0.15s; }
  .poster-btn:hover .poster-edit { opacity: 1; }
  .poster.ph { display: grid; place-items: center; background: rgb(255 255 255 / 0.1); }
  .hero-text { min-width: 0; }
  .kind { display: inline-flex; gap: 5px; align-items: center; font-size: 11px; font-weight: 650; opacity: 0.85; text-transform: uppercase; letter-spacing: 0.08em; }
  h2 { margin: 3px 0 3px; font-size: 21px; font-weight: 800; letter-spacing: -0.02em; line-height: 1.15; }
  .hero p { margin: 0; color: rgb(255 255 255 / 0.8); font-size: 12.5px; }

  .pathbar { display: flex; align-items: center; gap: 4px; padding: 8px 12px 8px 18px; border-bottom: 1px solid var(--line); }
  .path { flex: 1; min-width: 0; font-family: ui-monospace, "Cascadia Code", monospace; font-size: 12px; color: var(--text-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .ib { display: grid; place-items: center; width: 30px; height: 30px; border-radius: 8px; border: 0; background: transparent; color: var(--text-2); cursor: pointer; flex: none; }
  .ib:hover { background: var(--chip); color: var(--text); }
  .ib.accent { background: var(--accent); color: white; }
  .ib.hot { background: #f59e0b; color: #1a1205; }

  .install { display: flex; align-items: center; gap: 10px; margin: 10px 14px 0; padding: 9px 10px 9px 12px; border-radius: 10px; background: var(--chip); color: var(--muted); }
  .install.ok { background: rgb(16 185 129 / 0.1); color: var(--ok); }
  .play-band select { max-width: 100%; height: 24px; margin-top: 2px; border-radius: 6px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: inherit; font-size: 11.5px; }
  .it { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .it b { font-size: 13px; }
  .it span { font-size: 11.5px; color: var(--text-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sbtn { display: inline-flex; align-items: center; gap: 5px; height: 28px; padding: 0 10px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: inherit; font-size: 12px; font-weight: 600; cursor: pointer; white-space: nowrap; }
  .sbtn:hover { border-color: var(--accent); }
  .sbtn.primary { background: var(--accent); border-color: var(--accent); color: white; }
  .sbtn.danger { color: var(--err); }
  .sbtn.danger:hover { border-color: var(--err); background: var(--err-soft); }

  .warn { margin: 10px 14px 0; padding: 9px 12px; border-radius: 10px; background: var(--err-soft); color: var(--err); font-size: 12.5px; display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  .drawer > .pad { flex: 1; min-height: 0; padding: 12px 12px 14px; display: flex; flex-direction: column; }

  /* diffusion */
  .airing { display: flex; align-items: center; gap: 10px; margin: 10px 14px 0; padding: 9px 12px; border-radius: 10px; background: var(--accent-soft); color: var(--accent); font-size: 12.5px; }
  .ai-next { flex: 1; min-width: 0; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .ai-next em { font-style: normal; margin-left: 6px; color: var(--accent); font-weight: 700; }
  .ai-miss { flex: none; height: 26px; padding: 0 10px; border-radius: 999px; border: 0; background: var(--accent); color: white; font: inherit; font-size: 12px; font-weight: 700; cursor: pointer; }
  .eps li.ghost { border: 1.5px dashed var(--line-strong); background: none; }
  .eps li.ghost:hover { border-color: var(--accent); background: var(--accent-soft); }
  .eps li.ghost .ep { background: none; border: 1.5px dashed var(--accent); }
  .eps li.ghost .name { font-style: italic; color: var(--muted); }
  .eps li.ghost.upcoming { opacity: 0.6; }
  .eps li.ghost em.soon { color: var(--accent); background: var(--accent-soft); }
  .tabs { display: flex; gap: 4px; padding: 10px 14px 0; border-bottom: 1px solid var(--line); }
  .tabs button { display: inline-flex; align-items: center; gap: 6px; height: 34px; padding: 0 12px; border: 0; border-bottom: 2px solid transparent; margin-bottom: -1px; background: none; color: var(--muted); font: inherit; font-size: 13px; font-weight: 650; cursor: pointer; }
  .tabs button:hover { color: var(--text); }
  .tabs button.on { color: var(--accent); border-bottom-color: var(--accent); }
  .tn { min-width: 18px; padding: 0 5px; border-radius: 9px; background: var(--chip); font-size: 11px; color: var(--text-2); text-align: center; }
  .tn.bad { background: var(--err-soft); color: var(--err); }
  .ib.danger:hover { background: var(--err-soft); color: var(--err); }

  /* ---------- liste d'épisodes d'un groupe (même style que la liste de fichiers) ---------- */
  .eps-head { display: flex; align-items: baseline; gap: 2px; padding: 0 4px 8px; font-size: 13px; color: var(--muted); }
  .eps-head b { font-size: 17px; color: var(--text); font-weight: 800; }
  .eps-head em { font-style: normal; margin-left: 8px; padding: 1px 7px; border-radius: 6px; background: var(--chip); font-size: 11.5px; font-weight: 600; color: var(--text-2); }
  .resume { display: inline-flex; align-items: center; gap: 6px; height: 26px; margin-left: auto; padding: 0 10px; border-radius: 8px; border: 0; background: var(--accent); color: white; font: inherit; font-size: 12px; font-weight: 700; cursor: pointer; white-space: nowrap; align-self: center; }
  .resume:hover { filter: brightness(1.1); }
  .resume i { font-style: normal; font-weight: 600; opacity: 0.85; font-variant-numeric: tabular-nums; }
  .seen-n { display: inline-flex; align-items: center; gap: 5px; margin-left: auto; font-size: 12px; color: var(--muted); align-self: center; }
  .wbadge { display: inline-flex; align-items: center; gap: 4px; margin-right: 6px; padding: 1px 6px; border-radius: 5px; font-size: 10.5px; font-weight: 700; font-variant-numeric: tabular-nums; }
  .wbadge.seen { background: rgb(22 163 74 / 0.12); color: var(--ok); }
  .wbadge.mid { background: rgb(245 158 11 / 0.15); color: #d97706; }
  .eps li.seen .name { opacity: 0.55; }
  .eps { flex: 1; min-height: 0; overflow-y: auto; list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 1px; }
  .eps li { flex: none; display: flex; align-items: center; gap: 10px; min-height: 42px; padding: 5px 8px; border-radius: 9px; transition: background 0.1s; cursor: default; }
  .eps li:hover { background: var(--chip); }
  .eps .ep { flex: none; min-width: 34px; padding: 3px 7px; border-radius: 6px; background: var(--accent-soft); color: var(--accent); font-size: 11.5px; font-weight: 800; text-align: center; white-space: nowrap; font-variant-numeric: tabular-nums; }
  .eps li.done .ep { background: rgb(16 185 129 / 0.12); color: var(--ok); }
  .eps li.paused .ep { background: rgb(245 158 11 / 0.15); color: #d97706; }
  .eps .body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 5px; }
  .eps .name { font-size: 12.5px; color: var(--text-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .eps .bar { height: 4px; border-radius: 4px; background: var(--chip); overflow: hidden; }
  .eps .bar span { display: block; height: 100%; border-radius: 4px; background: linear-gradient(90deg, var(--accent), var(--accent-2)); transition: width 0.6s ease; }
  .eps li.paused .bar span { background: #f59e0b; }
  .eps .meta { display: inline-flex; align-items: center; gap: 6px; font-size: 11.5px; color: var(--muted); white-space: nowrap; font-variant-numeric: tabular-nums; }
  .eps .meta em { font-style: normal; color: var(--accent); font-weight: 700; padding: 1px 6px; border-radius: 5px; background: var(--accent-soft); }
  .eps .ok { display: grid; place-items: center; width: 16px; height: 16px; border-radius: 50%; background: var(--ok); color: white; }
  .eps .pz { color: #d97706; font-weight: 700; }
  .eps .bad { color: var(--err); font-weight: 700; }
  .eps .mini { opacity: 0.55; }
  .eps li:hover .mini { opacity: 1; }
  .eps .mini.play { width: 26px; height: 26px; border-radius: 50%; background: var(--chip); color: var(--text); padding-left: 2px; }
  .eps .mini.play:hover:not(:disabled) { background: var(--accent); color: white; }
  .eps .mini:disabled { opacity: 0.15; cursor: default; }
  .tip { margin: 6px 0 0; font-size: 11px; color: var(--muted); text-align: center; }

  .mini { display: grid; place-items: center; width: 26px; height: 26px; border-radius: 7px; border: 0; background: transparent; color: var(--muted); cursor: pointer; }
  .mini:hover, .mini.on { background: var(--chip); color: var(--text); }
  .mini.play { color: var(--accent); }
  .mini.play:hover { background: var(--accent); color: white; }
</style>
