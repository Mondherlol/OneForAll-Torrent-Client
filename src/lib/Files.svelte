<script>
  import Icon from "./Icon.svelte";
  import { getFiles, getWatch, openPath, setFilePriority, setWatched, trashTorrentFiles } from "./api.js";
  import { fmtSize, isPaused } from "./model.js";
  import { confirmAsk, flash, openMenu } from "./ui.svelte.js";

  let { torrent, fill = false, episodes = false, onaction = null } = $props();
  // Pause / reprise à portée de main, au-dessus de la liste
  const canToggle = $derived(!!onaction && torrent.progress < 1 && torrent.status !== "error");
  const paused = $derived(isPaused(torrent.status));

  let files = $state([]);
  let loaded = $state(false);
  let filter = $state("");
  let showSmall = $state(false);
  let anchor = null; // dernier fichier cliqué, pour Maj+clic

  const collator = new Intl.Collator("fr", { numeric: true, sensitivity: "base" });
  const VIDEO = /\.(mkv|mp4|avi|m4v|mov|wmv|webm|ts|m2ts)$/i;

  let stamp = 0; // une réponse partie avant le dernier clic est ignorée
  async function load() {
    const started = Date.now();
    try {
      const f = await getFiles(torrent.hash);
      if (started < stamp) return;
      files = f.sort((a, b) => collator.compare(a.name, b.name));
      loadWatch();
    } catch (e) {
      flash(String(e));
    } finally {
      loaded = true;
    }
  }

  $effect(() => {
    torrent.hash;
    load();
    const id = setInterval(() => !document.hidden && load(), 3000);
    return () => clearInterval(id);
  });

  // Les vidéos sont l'essentiel : les petits fichiers à côté (.nfo, sous-titres, 1 Ko…) sont masqués par défaut
  const videoCount = $derived(files.filter((f) => VIDEO.test(f.name)).length);
  const hideSmall = $derived(!showSmall && videoCount >= 2 && videoCount < files.length);
  const base = $derived(hideSmall ? files.filter((f) => VIDEO.test(f.name)) : files);
  const shown = $derived(filter ? base.filter((f) => f.name.toLowerCase().includes(filter.toLowerCase())) : base);
  const selected = $derived(base.filter((f) => f.priority > 0));
  const hasUnchecked = $derived(base.length > 1 && base.some((f) => f.priority === 0));
  const selSize = $derived(selected.reduce((s, f) => s + f.size, 0));
  const leftovers = $derived(files.filter((f) => f.priority === 0 && f.progress > 0));
  const leftSize = $derived(leftovers.reduce((s, f) => s + f.size * f.progress, 0));
  const unit = $derived(hideSmall ? (episodes ? "épisodes" : "vidéos") : "fichiers");

  // Temps restant estimé par fichier, en supposant un téléchargement dans l'ordre
  const etas = $derived.by(() => {
    const out = new Map();
    const speed = torrent.dlspeed;
    if (!speed) return out;
    let remaining = 0;
    for (const f of files) {
      if (f.priority === 0 || f.progress >= 1) continue;
      remaining += f.size * (1 - f.progress);
      out.set(f.index, remaining / speed);
    }
    return out;
  });
  const fmtEta = (s) => (s < 60 ? "< 1 min" : s < 3600 ? `${Math.round(s / 60)} min` : `${Math.floor(s / 3600)} h ${String(Math.round((s % 3600) / 60)).padStart(2, "0")}`);

  /** « Dossier/Black.Clover.E006.MULTi.1080p.mkv » → { ep: "É6", name: "Black Clover E006 MULTi 1080p" } */
  function pretty(f) {
    const file = f.name.split("/").pop();
    let stem = file.replace(/\.[a-z0-9]{2,4}$/i, "");
    if ((stem.match(/[._]/g) ?? []).length > (stem.match(/ /g) ?? []).length) stem = stem.replace(/[._]/g, " ");
    let m = stem.match(/S(\d{1,2})\s?E(\d{1,4})/i);
    let ep = m ? `S${+m[1]} · É${+m[2]}` : null;
    if (!ep && (m = stem.match(/\bE(?:p|pisode)?\s?(\d{1,4})\b/i))) ep = `É${+m[1]}`;
    if (!ep && (m = stem.match(/\s-\s(\d{1,4})(?:\s|$)/))) ep = `É${+m[1]}`;
    return { ep, name: stem };
  }

  const fullPath = (f) => `${torrent.save_path.replace(/[\\/]+$/, "")}\\${f.name.replaceAll("/", "\\")}`;

  // ---------- vu / en cours (historique de VLC) ----------
  let watch = $state({});
  async function loadWatch() {
    const vids = files.filter((f) => VIDEO.test(f.name));
    if (!vids.length) return;
    try {
      watch = await getWatch(vids.map(fullPath));
    } catch {}
  }
  const w = (f) => watch[fullPath(f)];
  const fmtPos = (ms) => {
    const s = Math.floor(ms / 1000);
    const h = Math.floor(s / 3600);
    const m = String(Math.floor((s % 3600) / 60)).padStart(h ? 2 : 1, "0");
    return `${h ? h + ":" : ""}${m}:${String(s % 60).padStart(2, "0")}`;
  };
  // Où j'en suis : l'épisode commencé le plus récemment, sinon celui qui suit le dernier vu
  const resume = $derived.by(() => {
    const vids = files.filter((f) => VIDEO.test(f.name) && f.priority > 0);
    const started = vids.filter((f) => w(f)?.pos_ms > 0 && !w(f)?.watched).sort((a, b) => w(b).at - w(a).at)[0];
    if (started) return { f: started, pos: w(started).pos_ms };
    let last = -1;
    vids.forEach((f, i) => w(f)?.watched && (last = i));
    if (last < 0) return null;
    const next = vids.slice(last + 1).find((f) => !w(f)?.watched && f.progress > 0);
    return next ? { f: next, pos: 0 } : null;
  });
  const seenCount = $derived(files.filter((f) => w(f)?.watched).length);
  async function markSeen(list, seen) {
    try {
      await setWatched(list.map(fullPath), seen);
      loadWatch();
    } catch (e) {
      flash(String(e));
    }
  }

  async function setPrio(list, prio) {
    if (!list.length) return;
    stamp = Date.now();
    for (const f of list) f.priority = prio; // affichage immédiat
    try {
      await setFilePriority(torrent.hash, list.map((f) => f.index), prio);
    } catch (e) {
      flash(String(e));
    }
    load();
  }

  /** Clic sur une case : Maj+clic applique le même état à toute la plage. */
  function clickBox(e, f) {
    // On laisse la case changer d'elle-même : bloquer le clic la faisait revenir en arrière
    const want = e.currentTarget.checked ? 1 : 0;
    const i = shown.indexOf(f);
    if (e.shiftKey && anchor != null && shown[anchor]) {
      const [a, b] = anchor < i ? [anchor, i] : [i, anchor];
      setPrio(shown.slice(a, b + 1).filter((x) => (x.priority > 0 ? 0 : 1) === want), want);
    } else {
      setPrio([f], want);
    }
    anchor = i;
  }

  /** Coche les N suivants (après le dernier coché). */
  function addNext(n) {
    let last = -1;
    shown.forEach((f, i) => f.priority > 0 && (last = i));
    const next = shown.slice(last + 1).filter((f) => f.priority === 0).slice(0, n);
    if (!next.length) return flash("Plus rien à ajouter");
    setPrio(next, 1);
    flash(`${next.length} ${next.length > 1 ? unit : unit.replace(/s$/, "")} ajouté${next.length > 1 ? "s" : ""} au téléchargement`);
  }

  async function freeSpace() {
    const ok = await confirmAsk(
      `Supprimer ${leftovers.length} fichier${leftovers.length > 1 ? "s" : ""} décoché${leftovers.length > 1 ? "s" : ""} du disque ?`,
      `Environ ${fmtSize(leftSize)} envoyés à la Corbeille. Les fichiers cochés ne sont pas touchés.`,
      "Mettre à la Corbeille",
      true,
    );
    if (!ok) return;
    try {
      const [n, bytes] = await trashTorrentFiles(torrent.hash, leftovers.map((f) => f.index));
      flash(n ? `${n} fichier${n > 1 ? "s" : ""} à la Corbeille (${fmtSize(bytes)})` : "Aucun fichier à supprimer");
      load();
    } catch (e) {
      flash(String(e));
    }
  }

  function open(f) {
    openPath(fullPath(f)).catch((e) => flash(String(e)));
  }

  function menu(e, f) {
    openMenu(e, [
      { label: "Lire / ouvrir", icon: "play", run: () => open(f), disabled: f.progress <= 0 },
      { label: f.priority > 0 ? "Ne pas télécharger" : "Télécharger", icon: f.priority > 0 ? "close" : "down", run: () => setPrio([f], f.priority > 0 ? 0 : 1) },
      { label: "Priorité haute", icon: "up", run: () => setPrio([f], 7), checked: f.priority === 7 },
      { label: "Priorité normale", icon: "list", run: () => setPrio([f], 1), checked: f.priority === 1 },
      VIDEO.test(f.name) && "-",
      VIDEO.test(f.name) && (w(f)?.watched
        ? { label: "Marquer comme pas vu", icon: "eye", run: () => markSeen([f], false) }
        : { label: "Marquer comme vu", icon: "eye", run: () => markSeen([f], true) }),
      VIDEO.test(f.name) && { label: "Marquer vus jusqu'ici", icon: "check", run: () => markSeen(shown.slice(0, shown.indexOf(f) + 1).filter((x) => VIDEO.test(x.name)), true) },
    ]);
  }
</script>

<div class="files" class:fill>
  {#if !loaded}
    <p class="muted pad">Chargement des fichiers…</p>
  {:else}
    <div class="toolbar">
      <div class="count">
        <b>{selected.length}</b><span>/{base.length} {unit}</span>
        <em>{fmtSize(selSize)}</em>
      </div>
      {#if resume}
        <button class="resume" title="Lancer dans VLC (il reprend là où tu t'es arrêté)" onclick={() => open(resume.f)}>
          <Icon name="play" size={11} />{resume.pos ? "Reprendre" : "Suivant"} : {pretty(resume.f).ep ?? pretty(resume.f).name}{#if resume.pos}<em>{fmtPos(resume.pos)}</em>{/if}
        </button>
      {:else if seenCount}
        <span class="seen-n"><Icon name="eye" size={12} />{seenCount} vu{seenCount > 1 ? "s" : ""}</span>
      {/if}
      <span class="spacer"></span>
      {#if hasUnchecked}
        <div class="group" title="Ajouter les suivants au téléchargement">
          <button onclick={() => addNext(1)}>+1</button>
          <button onclick={() => addNext(5)}>+5</button>
        </div>
      {/if}
      {#if base.length > 1}
        <div class="group">
          {#if hasUnchecked}<button title="Tout cocher" onclick={() => setPrio(shown.filter((f) => f.priority === 0), 1)}><Icon name="check" size={13} />Tout</button>{/if}
          <button title="Tout décocher" onclick={() => setPrio(shown.filter((f) => f.priority > 0), 0)}><Icon name="close" size={13} />Aucun</button>
        </div>
      {/if}
    </div>

    {#if files.length > 6 || leftovers.length || canToggle}
      <div class="toolbar sub">
        {#if files.length > 6}
          <label class="search"><Icon name="search" size={13} /><input bind:value={filter} placeholder="Filtrer…" spellcheck="false" /></label>
        {/if}
        {#if videoCount >= 2 && videoCount < files.length}
          <button class="toggle" class:on={!showSmall} onclick={() => (showSmall = !showSmall)} title="Masquer .nfo, sous-titres et autres petits fichiers">
            {showSmall ? "Tous les fichiers" : "Vidéos seulement"}
          </button>
        {/if}
        <span class="spacer"></span>
        {#if canToggle}
          <button class="toggle state" class:hot={paused} onclick={() => onaction(paused ? "resume" : "pause", [torrent])}>
            <Icon name={paused ? "play" : "pause"} size={12} />{paused ? "Reprendre" : "Pause"}
          </button>
        {/if}
        {#if leftovers.length}
          <button class="danger" onclick={freeSpace}><Icon name="trash" size={12} />Libérer {fmtSize(leftSize)}</button>
        {/if}
      </div>
    {/if}

    <ul>
      {#each shown as f (f.index)}
        {@const pct = Math.floor(f.progress * 1000) / 10}
        {@const p = pretty(f)}
        {@const done = f.progress >= 1}
        {@const wf = w(f)}
        <li class:off={f.priority === 0} class:done class:seen={wf?.watched} oncontextmenu={(e) => menu(e, f)} ondblclick={() => f.progress > 0 && open(f)}>
          <label class="check" title={f.priority > 0 ? "Ne pas télécharger" : "Télécharger"}>
            <input type="checkbox" checked={f.priority > 0} onclick={(e) => clickBox(e, f)} />
            <span class="box"><Icon name="check" size={12} /></span>
          </label>
          {#if p.ep}<span class="ep">{p.ep}</span>{/if}
          <div class="body">
            <span class="name" title={f.name}>{p.name}</span>
            {#if !done && (f.priority > 0 || f.progress > 0)}
              <div class="bar"><span style="width:{pct}%"></span></div>
            {/if}
          </div>
          <span class="meta">
            {#if wf?.watched}
              <span class="wbadge seen" title="Vu (d'après VLC)"><Icon name="eye" size={11} />Vu</span>
            {:else if wf?.pos_ms > 0}
              <span class="wbadge mid" title="Arrêté à {fmtPos(wf.pos_ms)} dans VLC"><Icon name="pause" size={9} />{fmtPos(wf.pos_ms)}</span>
            {/if}
            {#if done}
              <span class="ok"><Icon name="check" size={12} /></span>{fmtSize(f.size)}
            {:else}
              {#if etas.has(f.index)}<em>≈ {fmtEta(etas.get(f.index))}</em>{/if}
              {f.priority > 0 || f.progress > 0 ? `${pct}% · ` : ""}{fmtSize(f.size)}
            {/if}
          </span>
          <button class="play" title="Lire" disabled={f.progress <= 0} onclick={() => open(f)}><Icon name="play" size={11} /></button>
        </li>
      {/each}
    </ul>
    {#if shown.length > 1}
      <p class="tip">Maj+clic sur les cases : cocher une plage · double-clic : lire · clic droit : priorité</p>
    {/if}
  {/if}
</div>

<style>
  .files { display: flex; flex-direction: column; gap: 8px; }
  .muted { color: var(--muted); font-size: 12px; margin: 0; }
  .spacer { flex: 1; }
  .toolbar { display: flex; align-items: center; gap: 8px; padding: 0 2px; }
  .count { display: flex; align-items: baseline; gap: 2px; font-size: 13px; color: var(--muted); }
  .count b { font-size: 17px; color: var(--text); font-weight: 800; }
  .count em { font-style: normal; margin-left: 8px; padding: 1px 7px; border-radius: 6px; background: var(--chip); font-size: 11.5px; font-weight: 600; color: var(--text-2); }

  .group { display: inline-flex; padding: 2px; border-radius: 9px; background: var(--chip); }
  .group button { display: inline-flex; align-items: center; gap: 4px; height: 26px; padding: 0 10px; border: 0; border-radius: 7px; background: none; color: var(--text-2); font: inherit; font-size: 12px; font-weight: 700; cursor: pointer; }
  .group button:hover { background: var(--surface); color: var(--accent); box-shadow: var(--shadow-sm); }

  .toolbar.sub { gap: 6px; }
  .search { display: flex; align-items: center; gap: 6px; height: 28px; padding: 0 9px; width: 180px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--muted); }
  .search:focus-within { border-color: var(--accent); }
  .search input { flex: 1; min-width: 0; border: 0; outline: none; background: none; color: var(--text); font: inherit; font-size: 12px; }
  .toggle, .danger { display: inline-flex; align-items: center; gap: 5px; height: 28px; padding: 0 10px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text-2); font: inherit; font-size: 12px; font-weight: 600; cursor: pointer; }
  .toggle.on { border-color: var(--accent); color: var(--accent); background: var(--accent-soft); }
  .resume { display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 10px; margin-left: 10px; border-radius: 8px; border: 0; background: var(--accent); color: white; font: inherit; font-size: 12px; font-weight: 700; cursor: pointer; white-space: nowrap; }
  .resume:hover { filter: brightness(1.1); }
  .resume em { font-style: normal; font-weight: 600; opacity: 0.85; font-variant-numeric: tabular-nums; }
  .seen-n { display: inline-flex; align-items: center; gap: 5px; margin-left: 10px; font-size: 12px; color: var(--muted); }
  .wbadge { display: inline-flex; align-items: center; gap: 4px; margin-right: 6px; padding: 1px 6px; border-radius: 5px; font-size: 10.5px; font-weight: 700; font-variant-numeric: tabular-nums; }
  .wbadge.seen { background: rgb(22 163 74 / 0.12); color: var(--ok); }
  .wbadge.mid { background: rgb(245 158 11 / 0.15); color: #d97706; }
  li.seen .name { opacity: 0.55; }
  .state.hot { color: #b45309; border-color: rgb(245 158 11 / 0.45); background: rgb(245 158 11 / 0.12); }
  .danger { color: var(--err); border-color: rgb(225 29 72 / 0.3); }
  .danger:hover { background: var(--err-soft); }

  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 1px; max-height: 460px; overflow-y: auto; }
  li { display: flex; align-items: center; gap: 10px; min-height: 40px; padding: 5px 8px; border-radius: 9px; transition: background 0.1s; }
  li:hover { background: var(--chip); }
  li.off .name { color: var(--muted); }
  li.off .ep { opacity: 0.45; }
  .ep { flex: none; min-width: 34px; padding: 3px 7px; border-radius: 6px; background: var(--accent-soft); color: var(--accent); font-size: 11.5px; font-weight: 800; text-align: center; font-variant-numeric: tabular-nums; }
  li.done .ep { background: rgb(16 185 129 / 0.12); color: var(--ok); }
  .body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 5px; }
  .name { font-size: 12.5px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .bar { height: 4px; border-radius: 4px; background: var(--chip); overflow: hidden; }
  .bar span { display: block; height: 100%; border-radius: 4px; background: linear-gradient(90deg, var(--accent), var(--accent-2)); transition: width 0.6s ease; }
  li.off .bar span { background: var(--muted); }
  .meta { display: inline-flex; align-items: center; gap: 6px; font-size: 11.5px; color: var(--muted); white-space: nowrap; font-variant-numeric: tabular-nums; }
  .meta em { font-style: normal; color: var(--accent); font-weight: 700; padding: 1px 6px; border-radius: 5px; background: var(--accent-soft); }
  .ok { display: grid; place-items: center; width: 16px; height: 16px; border-radius: 50%; background: var(--ok); color: white; }
  .play { display: grid; place-items: center; width: 26px; height: 26px; padding-left: 2px; border-radius: 50%; border: 0; background: var(--chip); color: var(--text); cursor: pointer; flex: none; opacity: 0.6; transition: opacity 0.1s; }
  li:hover .play:not(:disabled) { opacity: 1; }
  .play:hover:not(:disabled) { background: var(--accent); color: white; }
  .play:disabled { opacity: 0.15; cursor: default; }
  .tip { margin: 0; padding: 0 4px; font-size: 11px; color: var(--muted); text-align: center; }

  .files.fill { height: 100%; min-height: 0; }
  .files.fill ul { max-height: none; flex: 1; min-height: 0; }

  .check { position: relative; display: grid; place-items: center; cursor: pointer; flex: none; }
  .check input { position: absolute; opacity: 0; width: 18px; height: 18px; margin: 0; cursor: pointer; }
  .box { width: 18px; height: 18px; border-radius: 5px; border: 1.5px solid var(--line-strong); display: grid; place-items: center; color: transparent; transition: all 0.12s; }
  .check input:checked + .box { background: var(--accent); border-color: var(--accent); color: white; }
  .check input:focus-visible + .box { box-shadow: 0 0 0 3px var(--ring); }
</style>
