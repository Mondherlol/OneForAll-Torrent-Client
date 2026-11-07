<script>
  import { fly, fade } from "svelte/transition";
  import Icon from "./Icon.svelte";
  import Files from "./Files.svelte";
  import Sources from "./Sources.svelte";
  import { KINDS, STATUS_LABEL, fmtEta, fmtSize, fmtSpeed, hueOf, isDone, isPaused, partialLabel, subtitle } from "./model.js";
  import { arts } from "./ui.svelte.js";

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
  const isVideo = $derived(g.kind === "film" || isShow);
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
      {#if anyRunning}
        <button class="big" title={single ? "Mettre en pause" : "Tout mettre en pause"} onclick={() => onaction("pause", incomplete)}><Icon name="pause" size={20} /></button>
      {:else if anyPaused}
        <button class="big hot" title={single ? "Reprendre" : "Tout reprendre"} onclick={() => onaction("resume", incomplete)}><Icon name="play" size={20} /></button>
      {:else if canPlay}
        <button class="big play" title="Lire" onclick={() => onaction("play", g.items)}><Icon name="play" size={20} /></button>
      {/if}
      <button class="icon-btn close" title="Fermer (Échap)" onclick={onclose}><Icon name="close" size={15} /></button>
    </div>
    <div class="hero-row">
      {#if art && posterOk}
        <img class="poster" src={art.url} alt="" onerror={() => (posterOk = false)} />
      {:else}
        <span class="poster ph"><Icon name={g.kind} size={30} /></span>
      {/if}
      <div class="hero-text">
        <span class="kind"><Icon name={g.kind} size={12} />{kindLabel}</span>
        <h2>{g.title}</h2>
        <p>{[subtitle(g), g.partial ? `${fmtSize(g.size)} sur ${fmtSize(g.totalSize)} · ${partialLabel(g)}` : fmtSize(g.size)].filter(Boolean).join(" · ")}</p>
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
    <div class="pad"><Files torrent={it} fill episodes={g.kind === "series" || g.kind === "anime"} /></div>
  {:else}
    <!-- Plusieurs torrents (un par épisode) : affichés comme UN seul torrent avec sa liste d'épisodes -->
    {@const doneCount = g.items.filter((i) => i.progress >= 1).length}
    <div class="pad">
      <div class="eps-head">
        <b>{doneCount}</b><span>/{g.items.length} {isShow ? "épisodes" : "éléments"} téléchargés</span>
        <em>{fmtSize(g.size)}</em>
      </div>
      <ul class="eps">
        {#each g.items as it (it.hash)}
          {@const pct = Math.floor(it.progress * 1000) / 10}
          {@const done = it.progress >= 1}
          {@const paused = isPaused(it.status) && !done}
          <li
            class:done
            class:paused
            class:err={it.status === "error"}
            oncontextmenu={(e) => onitemmenu(e, it)}
            ondblclick={(e) => !e.target.closest("button") && it.progress > 0 && onaction(isVideo ? "play" : "reveal", isVideo ? [it] : it)}
          >
            {#if epLabel(it)}<span class="ep">{epLabel(it)}</span>{/if}
            <div class="body">
              <span class="name" title={it.name}>{prettyName(it.name)}</span>
              {#if !done}<div class="bar"><span style="width:{pct}%"></span></div>{/if}
            </div>
            <span class="meta">
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
            <button class="mini" title="Plus d'actions" onclick={(e) => onitemmenu(e, it)}><Icon name="more" size={14} /></button>
          </li>
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
  .hero-actions { position: absolute; top: 14px; right: 14px; z-index: 2; display: flex; align-items: flex-start; gap: 8px; }
  .close { width: 30px; height: 30px; background: rgb(8 10 16 / 0.5); color: white; border-color: rgb(255 255 255 / 0.15); }
  .close:hover { color: white; border-color: white; }
  .big { display: grid; place-items: center; width: 50px; height: 50px; margin-top: 8px; border-radius: 50%; border: 0; background: white; color: #111; cursor: pointer; box-shadow: 0 8px 24px rgb(0 0 0 / 0.4); transition: scale 0.12s; }
  .big:hover { scale: 1.07; }
  .big.hot { background: #f59e0b; color: #1a1205; }
  .big.play { background: var(--accent); color: white; padding-left: 3px; }
  .hero-row { position: relative; display: flex; gap: 14px; align-items: center; padding-right: 110px; }
  .hero-progress { position: relative; margin-top: 14px; display: flex; flex-direction: column; gap: 6px; }
  .hp-bar { height: 6px; border-radius: 6px; background: rgb(255 255 255 / 0.18); overflow: hidden; }
  .hp-bar span { display: block; height: 100%; border-radius: 6px; background: linear-gradient(90deg, #2b88ff, #4cc2ff); box-shadow: 0 0 12px #4aa8ff; transition: width 0.6s ease; }
  .hero-progress.paused .hp-bar span { background: #f59e0b; box-shadow: none; }
  .hp-text { display: flex; gap: 12px; align-items: baseline; font-size: 12.5px; color: rgb(255 255 255 / 0.8); }
  .hp-text b { font-size: 15px; color: white; font-variant-numeric: tabular-nums; }
  .hero-progress.paused .hp-text span:first-of-type { color: #fbbf24; font-weight: 700; }
  .poster { width: 64px; aspect-ratio: 2 / 3; object-fit: cover; border-radius: 8px; box-shadow: 0 8px 20px rgb(0 0 0 / 0.5); flex: none; }
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
