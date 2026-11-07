<script>
  // Choix des fichiers d'un torrent tout juste ajouté (ajouté sans démarrer).
  // Un seul fichier → on lance directement ; sinon on laisse choisir.
  import { fade, scale } from "svelte/transition";
  import Icon from "./Icon.svelte";
  import { getCategories, getDisks, getFiles, setCategory, setFilePriority, torrentAction } from "./api.js";
  import { KIND_LABEL, fmtSize } from "./model.js";
  import { arts, flash, wantArt } from "./ui.svelte.js";

  let { hash, torrents, onclose, onfreespace } = $props();

  let files = $state([]);
  let phase = $state("waiting"); // waiting → choose → starting
  let sel = $state(new Set());
  let showSmall = $state(false);
  let filter = $state("");
  let disks = $state([]);
  let waited = $state(0);

  const VIDEO = /\.(mkv|mp4|avi|m4v|mov|wmv|webm|ts|m2ts)$/i;
  const collator = new Intl.Collator("fr", { numeric: true, sensitivity: "base" });

  const t = $derived(torrents.find((x) => x.hash === hash));
  $effect(() => {
    if (t) wantArt({ artKey: t.group_key, kind: t.kind, title: t.title, year: t.year });
  });
  const art = $derived(t ? arts[t.group_key] : null);

  // On attend que qBittorrent connaisse la liste des fichiers (immédiat pour un .torrent, quelques secondes pour un magnet)
  $effect(() => {
    let stop = false;
    getDisks().then((d) => (disks = d)).catch(() => {});
    const tick = async () => {
      if (stop || phase !== "waiting") return;
      try {
        const f = await getFiles(hash);
        if (f.length) return ready(f);
      } catch {}
      waited++;
      setTimeout(tick, 700);
    };
    tick();
    return () => (stop = true);
  });

  async function ready(f) {
    files = f.sort((a, b) => collator.compare(a.name, b.name));
    // Toujours affiché (même pour un seul fichier) : on vérifie le type, l'image, la catégorie
    sel = new Set(files.map((x) => x.index));
    phase = "choose";
  }

  // Catégorie (= type rangé + dossier) modifiable avant de lancer
  let categories = $state([]);
  $effect(() => {
    getCategories().then((c) => (categories = c)).catch(() => {});
  });
  async function changeCategory(cat) {
    try {
      await setCategory([hash], cat);
      flash(cat ? `Rangé dans « ${cat} »` : "Sans catégorie");
    } catch (e) {
      flash(String(e));
    }
  }

  // ---------- affichage ----------
  const videoCount = $derived(files.filter((f) => VIDEO.test(f.name)).length);
  const hideSmall = $derived(!showSmall && videoCount >= 2 && videoCount < files.length);
  const root = $derived.by(() => {
    if (files.length < 2) return "";
    const first = files[0].name.split("/")[0];
    return files.every((f) => f.name.startsWith(first + "/")) ? first + "/" : "";
  });
  const visible = $derived(
    files.filter((f) => (!hideSmall || VIDEO.test(f.name)) && (!filter || f.name.toLowerCase().includes(filter.toLowerCase()))),
  );
  // Regroupement par dossier (Saison 1, Saison 2…)
  const groups = $derived.by(() => {
    const m = new Map();
    for (const f of visible) {
      const rel = f.name.slice(root.length).split("/");
      const folder = rel.length > 1 ? rel.slice(0, -1).join(" / ") : "";
      if (!m.has(folder)) m.set(folder, []);
      m.get(folder).push(f);
    }
    return [...m.entries()].map(([folder, list]) => ({ folder, list }));
  });

  const selFiles = $derived(files.filter((f) => sel.has(f.index)));
  const selSize = $derived(selFiles.reduce((s, f) => s + f.size, 0));
  const selVisible = $derived(visible.filter((f) => sel.has(f.index)).length);
  const drive = $derived(disks.find((d) => t?.save_path?.toUpperCase().startsWith(d.mount.replace(/\\$/, "").toUpperCase())));
  const tooBig = $derived(drive && selSize > drive.free);
  const unit = $derived(hideSmall ? (t?.kind === "series" || t?.kind === "anime" ? "épisodes" : "vidéos") : "fichiers");

  function pretty(f) {
    let stem = f.name.split("/").pop().replace(/\.[a-z0-9]{2,4}$/i, "");
    if ((stem.match(/[._]/g) ?? []).length > (stem.match(/ /g) ?? []).length) stem = stem.replace(/[._]/g, " ");
    let m = stem.match(/S(\d{1,2})\s?E(\d{1,4})/i);
    let ep = m ? `S${+m[1]} · É${+m[2]}` : null;
    if (!ep && (m = stem.match(/\bE(?:p|pisode)?\s?(\d{1,4})\b/i))) ep = `É${+m[1]}`;
    if (!ep && (m = stem.match(/\s-\s(\d{1,4})(?:\s|$)/))) ep = `É${+m[1]}`;
    return { ep, name: stem };
  }

  // ---------- sélection ----------
  function toggle(f) {
    const s = new Set(sel);
    s.has(f.index) ? s.delete(f.index) : s.add(f.index);
    sel = s;
  }
  function setList(list, on) {
    const s = new Set(sel);
    for (const f of list) on ? s.add(f.index) : s.delete(f.index);
    sel = s;
  }
  const allOn = (list) => list.every((f) => sel.has(f.index));
  const someOn = (list) => list.some((f) => sel.has(f.index));
  /** Seulement les N premiers (dans l'ordre des épisodes) */
  function firstN(n) {
    const order = visible;
    sel = new Set(order.slice(0, n).map((f) => f.index));
    // les petits fichiers masqués suivent leur dossier (voir start)
  }

  // ---------- validation ----------
  async function start(auto = false) {
    phase = "starting";
    try {
      if (!auto) {
        // Les petits fichiers masqués (.nfo, sous-titres) suivent : gardés si une vidéo du même dossier est choisie
        const folderOf = (f) => f.name.split("/").slice(0, -1).join("/");
        const chosenFolders = new Set(selFiles.filter((f) => VIDEO.test(f.name)).map(folderOf));
        const keep = (f) => (hideSmall && !VIDEO.test(f.name) ? chosenFolders.has(folderOf(f)) : sel.has(f.index));
        const off = files.filter((f) => !keep(f)).map((f) => f.index);
        if (off.length === files.length) {
          phase = "choose";
          return flash("Choisis au moins un fichier");
        }
        if (off.length) await setFilePriority(hash, off, 0);
      }
      await torrentAction([hash], "resume");
      flash(auto ? `Téléchargement lancé : ${t?.title ?? "torrent"}` : `Téléchargement lancé · ${selVisible} ${unit}`);
      onclose();
    } catch (e) {
      flash(String(e));
      phase = "choose";
    }
  }

  async function cancel() {
    try {
      await torrentAction([hash], "delete");
    } catch {}
    flash("Ajout annulé");
    onclose();
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && phase !== "starting" && cancel()} />

{#if phase !== "starting" || files.length > 1}
  <div class="backdrop" transition:fade={{ duration: 140 }} role="presentation"></div>
  <div class="modal" transition:scale={{ start: 0.97, duration: 160 }} role="dialog" aria-label="Choisir les fichiers">
    <header>
      {#if art?.url}
        <img class="poster" src={art.url} alt="" />
      {:else}
        <span class="poster ph"><Icon name={t?.kind ?? "other"} size={26} /></span>
      {/if}
      <div class="ht">
        <span class="kind">
          {t ? KIND_LABEL[t.kind] : "Nouveau torrent"}
          {#if t?.format}<span class="fmt">{t.format}</span>{/if}
          {#if t?.platform}<span class="plat">{t.platform}</span>{/if}
        </span>
        <h2 title={t?.name}>{t?.title ?? "Nouveau torrent"}</h2>
        {#if phase === "choose"}<p>{files.length} fichier{files.length > 1 ? "s" : ""} · {fmtSize(files.reduce((s, f) => s + f.size, 0))} au total</p>{/if}
        {#if t?.name}<p class="raw" title={t.name}>{t.name}</p>{/if}
      </div>
      {#if t}
        <label class="cat" title="Catégorie qBittorrent (et donc le dossier de destination)">
          <span>Ranger dans</span>
          <select value={t.category} onchange={(e) => changeCategory(e.currentTarget.value)}>
            <option value="">Aucune catégorie</option>
            {#each categories as c}<option value={c.name}>{c.name}</option>{/each}
          </select>
        </label>
      {/if}
    </header>

    {#if phase === "waiting"}
      <div class="wait">
        <span class="spinner"></span>
        <b>Récupération de la liste des fichiers…</b>
        <span>{waited > 12 ? "Peu de sources pour ce lien magnet, ça peut prendre un moment." : "Rien ne se télécharge tant que tu n'as pas choisi."}</span>
      </div>
    {:else}
      <div class="bar">
        <div class="quick">
          <button onclick={() => setList(visible, true)}>Tout</button>
          <button onclick={() => setList(visible, false)}>Rien</button>
          {#each [3, 5, 10] as n}
            {#if visible.length > n}<button onclick={() => firstN(n)}>{n} premiers</button>{/if}
          {/each}
        </div>
        <span class="spacer"></span>
        {#if files.length > 8}
          <label class="search"><Icon name="search" size={13} /><input bind:value={filter} placeholder="Filtrer…" spellcheck="false" /></label>
        {/if}
        {#if videoCount >= 2 && videoCount < files.length}
          <button class="toggle" class:on={!showSmall} onclick={() => (showSmall = !showSmall)}>{showSmall ? "Tous les fichiers" : "Vidéos seulement"}</button>
        {/if}
      </div>

      <div class="list">
        {#each groups as g (g.folder)}
          {#if g.folder}
            <button class="folder" onclick={() => setList(g.list, !allOn(g.list))}>
              <span class="box" class:on={allOn(g.list)} class:some={!allOn(g.list) && someOn(g.list)}>
                <Icon name={allOn(g.list) ? "check" : "close"} size={11} />
              </span>
              <Icon name="folder" size={14} />
              <b>{g.folder}</b>
              <em>{g.list.filter((f) => sel.has(f.index)).length}/{g.list.length} · {fmtSize(g.list.reduce((s, f) => s + f.size, 0))}</em>
            </button>
          {/if}
          {#each g.list as f (f.index)}
            {@const p = pretty(f)}
            <button class="file" class:off={!sel.has(f.index)} class:nested={!!g.folder} onclick={() => toggle(f)}>
              <span class="box" class:on={sel.has(f.index)}><Icon name="check" size={11} /></span>
              {#if p.ep}<span class="ep">{p.ep}</span>{/if}
              <span class="name" title={f.name}>{p.name}</span>
              <span class="size">{fmtSize(f.size)}</span>
            </button>
          {/each}
        {/each}
      </div>
    {/if}

    {#if phase !== "waiting" && tooBig}
      <!-- Pas assez de place sur le disque de destination -->
      <div class="space">
        <Icon name="alert" size={18} />
        <div>
          <b>Pas assez de place sur {drive.mount.replace(/\\$/, "")}</b>
          <span>Il manque {fmtSize(selSize - drive.free)} : {fmtSize(selSize)} choisis, {fmtSize(drive.free)} libres. Le téléchargement s'arrêtera quand le disque sera plein.</span>
        </div>
        <button class="btn" onclick={() => onfreespace?.()}><Icon name="drive" size={14} />Libérer de la place</button>
      </div>
    {/if}

    <footer>
      {#if phase !== "waiting"}
        <div class="sum">
          <b>{selVisible}</b> {unit} · <b>{fmtSize(selSize)}</b>
          {#if drive}<span class:bad={tooBig}> · {fmtSize(drive.free)} libres sur {drive.mount.replace(/\\$/, "")}</span>{/if}
        </div>
      {/if}
      <span class="spacer"></span>
      <button class="btn" onclick={cancel}>Annuler</button>
      {#if phase !== "waiting"}
        <button class="btn primary" class:warn-btn={tooBig} disabled={phase === "starting" || !selVisible} onclick={() => start()}>
          <Icon name="down" size={14} />{tooBig ? "Télécharger quand même" : "Télécharger"}
        </button>
      {/if}
    </footer>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 120; background: var(--scrim); backdrop-filter: blur(3px); }
  .modal {
    position: fixed;
    z-index: 121;
    left: 50%;
    top: 50%;
    translate: -50% -50%;
    width: min(720px, 94vw);
    max-height: 86vh;
    display: flex;
    flex-direction: column;
    border-radius: 18px;
    overflow: hidden;
    background: var(--bg-2);
    border: 1px solid var(--line-strong);
    box-shadow: var(--shadow-lg);
  }
  header { display: flex; gap: 14px; align-items: center; padding: 16px 18px; border-bottom: 1px solid var(--line); }
  .poster { width: 52px; aspect-ratio: 2 / 3; object-fit: cover; border-radius: 7px; box-shadow: var(--shadow-sm); flex: none; }
  .poster.ph { display: grid; place-items: center; background: var(--chip); color: var(--muted); }
  .ht { min-width: 0; }
  .kind { font-size: 11px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.08em; color: var(--accent); }
  h2 { margin: 2px 0; font-size: 18px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  header p { margin: 0; font-size: 12.5px; color: var(--muted); }
  header .raw { font-size: 11px; font-family: ui-monospace, "Cascadia Code", monospace; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 420px; opacity: 0.8; }
  .ht { flex: 1; }
  .kind { display: inline-flex; align-items: center; gap: 6px; }
  .fmt { padding: 1px 5px; border-radius: 4px; background: #e60012; color: white; font-size: 10px; letter-spacing: 0.04em; }
  .plat { padding: 1px 6px; border-radius: 4px; background: var(--chip); color: var(--text-2); font-size: 10px; letter-spacing: 0.04em; }
  .cat { display: flex; flex-direction: column; gap: 4px; flex: none; }
  .cat span { font-size: 11px; color: var(--muted); font-weight: 600; }
  .cat select { height: 32px; padding: 0 10px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: inherit; font-size: 13px; cursor: pointer; }

  .wait { display: flex; flex-direction: column; align-items: center; gap: 8px; padding: 40px 20px; text-align: center; }
  .wait span:last-child { color: var(--muted); font-size: 12.5px; }
  .spinner { width: 26px; height: 26px; border-radius: 50%; border: 3px solid var(--line-strong); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }

  .bar { display: flex; align-items: center; gap: 8px; padding: 10px 14px; border-bottom: 1px solid var(--line); flex-wrap: wrap; }
  .quick { display: inline-flex; padding: 2px; border-radius: 9px; background: var(--chip); }
  .quick button { height: 26px; padding: 0 10px; border: 0; border-radius: 7px; background: none; color: var(--text-2); font: inherit; font-size: 12px; font-weight: 700; cursor: pointer; }
  .quick button:hover { background: var(--surface); color: var(--accent); box-shadow: var(--shadow-sm); }
  .spacer { flex: 1; }
  .search { display: flex; align-items: center; gap: 6px; height: 28px; padding: 0 9px; width: 160px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--muted); }
  .search input { flex: 1; min-width: 0; border: 0; outline: none; background: none; color: var(--text); font: inherit; font-size: 12px; }
  .toggle { height: 28px; padding: 0 10px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text-2); font: inherit; font-size: 12px; font-weight: 600; cursor: pointer; }
  .toggle.on { border-color: var(--accent); color: var(--accent); background: var(--accent-soft); }

  .list { flex: 1; min-height: 120px; overflow-y: auto; padding: 6px 8px 10px; }
  .folder, .file { display: flex; align-items: center; gap: 10px; width: 100%; border: 0; background: none; color: var(--text); font: inherit; text-align: left; cursor: pointer; border-radius: 8px; }
  .folder { margin-top: 8px; padding: 7px 8px; font-size: 13px; }
  .folder:hover, .file:hover { background: var(--chip); }
  .folder em { margin-left: auto; font-style: normal; font-size: 11.5px; color: var(--muted); }
  .file { min-height: 34px; padding: 4px 8px; }
  .file.nested { padding-left: 30px; }
  .file.off .name { color: var(--muted); }
  .file.off .ep { opacity: 0.45; }
  .ep { flex: none; min-width: 34px; padding: 2px 7px; border-radius: 6px; background: var(--accent-soft); color: var(--accent); font-size: 11.5px; font-weight: 800; text-align: center; white-space: nowrap; }
  .name { flex: 1; min-width: 0; font-size: 12.5px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .size { font-size: 11.5px; color: var(--muted); white-space: nowrap; font-variant-numeric: tabular-nums; }
  .box { flex: none; width: 18px; height: 18px; border-radius: 5px; border: 1.5px solid var(--line-strong); display: grid; place-items: center; color: transparent; transition: all 0.1s; }
  .box.on { background: var(--accent); border-color: var(--accent); color: white; }
  .box.some { border-color: var(--accent); background: var(--accent-soft); }

  .space { display: flex; align-items: center; gap: 12px; margin: 0 14px 4px; padding: 10px 12px; border-radius: 10px; background: var(--err-soft); color: var(--err); }
  .space div { flex: 1; display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .space b { font-size: 13px; }
  .space span { font-size: 12px; color: var(--text-2); }
  :global(.btn.primary.warn-btn) { background: var(--err); border-color: var(--err); }
  footer { display: flex; align-items: center; gap: 8px; padding: 12px 16px; border-top: 1px solid var(--line); }
  .sum { font-size: 12.5px; color: var(--muted); }
  .sum b { color: var(--text); }
  .sum .bad { color: var(--err); font-weight: 700; }
</style>
