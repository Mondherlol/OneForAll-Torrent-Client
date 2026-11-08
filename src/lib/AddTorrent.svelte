<script>
  import { fade, fly } from "svelte/transition";
  import { untrack } from "svelte";
  import { open as pickFiles } from "@tauri-apps/plugin-dialog";
  import Icon from "./Icon.svelte";
  import { openUrl, previewTorrent, searchTorrents, stageTorrents } from "./api.js";
  import { KINDS, KIND_LABEL, fmtAgo, fmtSize } from "./model.js";
  import { arts, flash, wantArt } from "./ui.svelte.js";

  let { torrents, onclose, onadded, initial = null } = $props();

  const TABS = [
    { id: "search", label: "Rechercher", icon: "search" },
    { id: "link", label: "Lien magnet", icon: "link" },
    { id: "file", label: "Fichier .torrent", icon: "torrent" },
  ];
  let tab = $state("search");
  const SOURCES = [
    { id: "c411", label: "C411" },
    { id: "nyaa", label: "Nyaa" },
    { id: "online-fix", label: "Online-Fix" },
  ];
  let source = $state(localStorage.getItem("oneforall.source") ?? "c411");
  $effect(() => localStorage.setItem("oneforall.source", source));
  let query = $state("");
  let results = $state(null);
  let loading = $state(false);
  let error = $state(null);
  let links = $state("");
  let added = $state(new Set());
  let inputEl = $state();

  $effect(() => {
    tab;
    setTimeout(() => inputEl?.focus(), 50);
  });

  const have = $derived(new Set(torrents.map((t) => t.hash.toLowerCase())));

  async function search() {
    if (!query.trim()) return;
    loading = true;
    error = null;
    selected = null;
    kindF = "all";
    try {
      results = await searchTorrents(query, source);
    } catch (e) {
      error = String(e);
      results = null;
    } finally {
      loading = false;
    }
  }

  // Les torrents sont ajoutés sans démarrer ; l'app propose ensuite de choisir les fichiers (onadded)
  async function addOne(r) {
    try {
      const res = await stageTorrents([r.url]);
      if (res.hashes.length) added = new Set([...added, r.url]);
      onadded(res);
    } catch (e) {
      flash(String(e));
    }
  }

  async function addLinks() {
    const urls = links.split(/\s+/).filter((l) => l.startsWith("magnet:") || l.startsWith("http"));
    if (!urls.length) return flash("Colle au moins un lien magnet:… ou https://….torrent");
    try {
      const res = await stageTorrents(urls);
      onadded(res);
      if (res.hashes.length) onclose();
    } catch (e) {
      flash(String(e));
    }
  }

  async function addFiles() {
    const picked = await pickFiles({ multiple: true, filters: [{ name: "Torrents", extensions: ["torrent"] }] });
    const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
    if (!paths.length) return;
    try {
      const res = await stageTorrents([], paths);
      onadded(res);
      if (res.hashes.length) onclose();
    } catch (e) {
      flash(String(e));
    }
  }

  const ts = (d) => Math.floor(Date.parse(d) / 1000) || 0;

  // ---------- tris & filtres ----------
  const SORTS = [
    { id: "seeders", label: "Plus de sources" },
    { id: "date", label: "Plus récents" },
    { id: "size-desc", label: "Plus gros" },
    { id: "size-asc", label: "Plus petits" },
    { id: "name", label: "Nom" },
  ];
  const LANGS = [
    { id: "all", label: "Toutes langues" },
    { id: "fr", label: "FR" },
    { id: "en", label: "EN" },
    { id: "raw", label: "Raw" },
  ];
  let sortBy = $state("seeders");
  let kindF = $state("all");
  let langF = $state(localStorage.getItem("oneforall.lang") ?? "all");
  $effect(() => localStorage.setItem("oneforall.lang", langF));
  let aliveOnly = $state(false);

  const kindsIn = $derived.by(() => {
    const counts = {};
    for (const r of results ?? []) counts[r.kind] = (counts[r.kind] ?? 0) + 1;
    return KINDS.filter((k) => counts[k.id]).map((k) => ({ ...k, n: counts[k.id] }));
  });
  const collator = new Intl.Collator("fr", { numeric: true, sensitivity: "base" });
  const shown = $derived.by(() => {
    let list = (results ?? []).filter(
      (r) =>
        (kindF === "all" || r.kind === kindF) &&
        (source !== "nyaa" || langF === "all" || r.lang === langF) &&
        (!aliveOnly || r.seeders !== 0) &&
        (!epCode || !epOnly || matchesEp(r.title)),
    );
    const cmp = {
      seeders: (a, b) => b.seeders - a.seeders,
      date: (a, b) => ts(b.date) - ts(a.date),
      "size-desc": (a, b) => b.size - a.size,
      "size-asc": (a, b) => a.size - b.size,
      name: (a, b) => collator.compare(a.title, b.title),
    }[sortBy];
    // Les sorties de la même équipe que le dernier épisode passent en premier
    return [...list].sort((a, b) => sameTeam(b) - sameTeam(a) || cmp(a, b));
  });

  // ---------- recherche préremplie (épisode d'une série) ----------
  let epCode = $state(null); // « S06E09 » ou « 09 »
  let epOnly = $state(true);
  let prefer = $state(null); // équipe de sortie du dernier épisode téléchargé
  $effect(() => {
    const init = initial;
    if (!init) return;
    // untrack : sinon chaque frappe dans le champ relancerait cette recherche
    untrack(() => {
      query = init.query;
      source = init.source;
      epCode = init.code ?? null;
      prefer = init.prefer ?? null;
      search();
    });
  });
  const matchesEp = (title) => {
    if (!epCode) return true;
    if (/^S\d+E\d+$/i.test(epCode)) return title.toUpperCase().replace(/[^A-Z0-9]/g, "").includes(epCode.toUpperCase());
    const n = String(+epCode);
    return new RegExp(`(?:^|[\\s_\\-\\[(.E])0*${n}(?:v\\d)?(?=[\\s_\\-\\])\\.]|$)`, "i").test(title);
  };
  const sameTeam = (r) => !!prefer && r.title.toLowerCase().includes(prefer.toLowerCase());

  // ---------- affiches (chargées quand la ligne devient visible) ----------
  function lazyArt(node, r) {
    const io = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) {
        wantArt({ artKey: r.art_key, kind: r.kind, title: r.clean || r.title, year: r.year });
        io.disconnect();
      }
    });
    io.observe(node);
    return { destroy: () => io.disconnect() };
  }

  // ---------- aperçu ----------
  let selected = $state(null);
  let preview = $state(null);
  async function select(r) {
    selected = r;
    wantArt({ artKey: r.art_key, kind: r.kind, title: r.clean || r.title, year: r.year });
    preview = { loading: true };
    const url = r.url;
    try {
      const data = await previewTorrent(url);
      if (selected?.url === url) preview = { data };
    } catch (e) {
      if (selected?.url === url) preview = { error: String(e) };
    }
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} />

<div class="backdrop" transition:fade={{ duration: 140 }} onclick={onclose} role="presentation"></div>
<div class="modal" transition:fly={{ y: 16, duration: 200 }} role="dialog" aria-label="Ajouter un torrent">
  <header>
    <h2>Ajouter un torrent</h2>
    <div class="seg">
      {#each TABS as t}
        <button class:on={tab === t.id} onclick={() => (tab = t.id)}><Icon name={t.icon} size={14} />{t.label}</button>
      {/each}
    </div>
    <button class="icon-btn" title="Fermer (Échap)" onclick={onclose}><Icon name="close" size={15} /></button>
  </header>

  {#if tab === "search"}
    <form class="bar" onsubmit={(e) => (e.preventDefault(), search())}>
      <label class="field">
        <Icon name="search" size={16} />
        <input bind:this={inputEl} bind:value={query} placeholder="Ex. : Black Clover 1080p" spellcheck="false" />
      </label>
      <div class="srcs" title="Où chercher">
        {#each SOURCES as s}
          <button type="button" class:on={source === s.id} onclick={() => ((source = s.id), results && search())}>{s.label}</button>
        {/each}
      </div>
      <button class="btn primary" disabled={loading || !query.trim()}>{loading ? "Recherche…" : "Rechercher"}</button>
    </form>

    {#if results?.length}
      <!-- Tris et filtres -->
      <div class="filters">
        <select bind:value={sortBy} title="Trier">
          {#each SORTS as s}<option value={s.id}>{s.label}</option>{/each}
        </select>
        <div class="chips">
          <button class:on={kindF === "all"} onclick={() => (kindF = "all")} title="Tous les types">Tout <em>{results.length}</em></button>
          {#each kindsIn as k}
            <button class:on={kindF === k.id} onclick={() => (kindF = k.id)} title={k.label}><Icon name={k.id} size={13} /><em>{k.n}</em></button>
          {/each}
        </div>
        {#if source === "nyaa"}
          <div class="chips" title="Langue">
            {#each LANGS as l}
              <button class:on={langF === l.id} onclick={() => (langF = l.id)}>{l.label}</button>
            {/each}
          </div>
        {/if}
        {#if epCode}
          <label class="alive ep"><input type="checkbox" bind:checked={epOnly} />Seulement l'épisode {epCode}</label>
        {/if}
        <label class="alive"><input type="checkbox" bind:checked={aliveOnly} />Avec sources</label>
      </div>
    {/if}

    <div class="body">
      <div class="results">
        {#if error}
          <p class="hint err">{error}</p>
        {:else if loading}
          <!-- Squelette pendant la recherche -->
          <ul class="skel">
            {#each Array(7) as _, i}
              <li style="--d:{i * 70}ms"><span class="sk-thumb"></span><span class="sk-lines"><i></i><i></i></span></li>
            {/each}
          </ul>
        {:else if results && !shown.length}
          <p class="hint">{results.length ? "Aucun résultat avec ces filtres." : `Aucun résultat pour « ${query} ».`}</p>
        {:else if results}
          <ul>
            {#each shown as r (r.url)}
              {@const already = have.has(r.hash) || added.has(r.url)}
              {@const art = arts[r.art_key]}
              <li class:sel={selected?.url === r.url} use:lazyArt={r}>
                <button class="row" onclick={() => select(r)}>
                  <span class="thumb k-{r.kind}">
                    {#if art?.url}<img src={art.url} alt="" loading="lazy" />{:else}<Icon name={r.kind} size={18} />{/if}
                  </span>
                  <span class="rbody">
                    <span class="rtitle" title={r.title}>{#if r.trusted}<i class="trusted" title="Uploader de confiance"></i>{/if}{r.title}</span>
                    <span class="rmeta">
                      <span class="kb k-{r.kind}">{KIND_LABEL[r.kind] ?? "Autre"}</span>
                      {#if r.lang}<span class="lb">{r.lang.toUpperCase()}</span>{/if}
                      {#if sameTeam(r)}<span class="team" title="Même équipe que ton dernier épisode">★ {prefer}</span>{/if}
                      {#if r.seeders >= 0}<b class="seed" class:dead={r.seeders === 0}>↑ {r.seeders}</b>
                      <span>↓ {r.leechers}</span>{/if}
                      <span>{fmtSize(r.size)}</span>
                      <span>{fmtAgo(ts(r.date))}</span>
                    </span>
                  </span>
                </button>
                {#if already}
                  <span class="done" title="Déjà ajouté"><Icon name="check" size={15} /></span>
                {:else}
                  <button class="dl" title="Télécharger" onclick={() => addOne(r)}><Icon name="down" size={15} /></button>
                {/if}
              </li>
            {/each}
          </ul>
        {:else}
          <p class="hint">Tape un titre puis Entrée. Clique sur un résultat pour voir son contenu avant de le télécharger.</p>
        {/if}
      </div>

      {#if selected}
        <!-- Aperçu du torrent avant téléchargement -->
        {@const art = arts[selected.art_key]}
        {@const already = have.has(selected.hash) || added.has(selected.url)}
        <aside class="preview" transition:fly={{ x: 20, duration: 160 }}>
          <div class="pv-head">
            {#if art?.url}<img class="pv-poster" src={art.url} alt="" />{:else}<span class="pv-poster ph"><Icon name={selected.kind} size={28} /></span>{/if}
            <div class="pv-ht">
              <span class="kb k-{selected.kind}">{KIND_LABEL[selected.kind] ?? "Autre"}{selected.year ? ` · ${selected.year}` : ""}</span>
              <h3>{selected.clean || selected.title}</h3>
              <div class="pv-stats">
                {#if selected.seeders >= 0}<span><b class="seed" class:dead={selected.seeders === 0}>{selected.seeders}</b> seeders</span>
                <span><b>{selected.leechers}</b> leechers</span>{/if}
                <span><b>{fmtSize(selected.size)}</b></span>
              </div>
            </div>
            <button class="pv-x" title="Fermer l'aperçu" onclick={() => (selected = null)}><Icon name="close" size={14} /></button>
          </div>
          <p class="pv-raw" title={selected.title}>{selected.title}</p>

          <div class="pv-files">
            {#if preview?.loading}
              <ul class="skel small">{#each Array(5) as _, i}<li style="--d:{i * 60}ms"><span class="sk-lines"><i></i></span></li>{/each}</ul>
            {:else if preview?.error}
              <p class="hint">{preview.error}</p>
            {:else if preview?.data}
              <h4>{preview.data.files.length} fichier{preview.data.files.length > 1 ? "s" : ""} · {fmtSize(preview.data.total)}</h4>
              <ul>
                {#each preview.data.files as f}
                  <li><span class="fname" title={f.path}>{f.path.split("/").pop()}</span><span class="fsize">{fmtSize(f.size)}</span></li>
                {/each}
              </ul>
            {/if}
          </div>

          <div class="pv-actions">
            {#if selected.page}<button class="btn" onclick={() => openUrl(selected.page)}><Icon name="open" size={14} />Voir sur le site</button>{/if}
            {#if already}
              <span class="done big"><Icon name="check" size={15} />Déjà ajouté</span>
            {:else}
              <button class="btn primary" onclick={() => addOne(selected)}><Icon name="down" size={14} />Télécharger</button>
            {/if}
          </div>
        </aside>
      {/if}
    </div>
  {:else if tab === "link"}
    <div class="pad">
      <textarea bind:this={inputEl} bind:value={links} placeholder="magnet:?xt=urn:btih:…  (un lien par ligne)" spellcheck="false"></textarea>
      <div class="actions"><button class="btn primary" onclick={addLinks}><Icon name="down" size={14} />Ajouter</button></div>
    </div>
  {:else}
    <div class="pad drop">
      <Icon name="torrent" size={38} />
      <p>Choisis un ou plusieurs fichiers .torrent.<br /><span>Astuce : tu peux aussi glisser-déposer des .torrent n'importe où sur la fenêtre.</span></p>
      <button class="btn primary" onclick={addFiles}><Icon name="folder" size={14} />Choisir des fichiers…</button>
    </div>
  {/if}
  <footer>Le type et le dossier sont choisis automatiquement (rangement automatique).</footer>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 60; background: var(--scrim); backdrop-filter: blur(3px); }
  .modal {
    position: fixed;
    z-index: 61;
    left: 50%;
    top: 6vh;
    translate: -50% 0;
    width: min(1120px, 96vw);
    height: min(680px, 86vh);
    display: flex;
    flex-direction: column;
    border-radius: 18px;
    overflow: hidden;
    background: var(--bg-2);
    border: 1px solid var(--line-strong);
    box-shadow: var(--shadow-lg);
  }
  header { display: flex; align-items: center; gap: 14px; padding: 16px 16px 12px 22px; border-bottom: 1px solid var(--line); }
  h2 { margin: 0; font-size: 17px; flex: 1; }
  .seg { display: inline-flex; padding: 3px; border-radius: 10px; background: var(--chip); }
  .seg button { display: inline-flex; align-items: center; gap: 6px; height: 30px; padding: 0 12px; border: 0; border-radius: 7px; background: none; color: var(--muted); font: inherit; font-size: 12.5px; font-weight: 600; cursor: pointer; }
  .seg button.on { background: var(--surface); color: var(--text); box-shadow: var(--shadow-sm); }

  .bar { display: flex; gap: 8px; align-items: center; padding: 14px 18px; }
  .field { flex: 1; display: flex; align-items: center; gap: 10px; height: 40px; padding: 0 12px; border-radius: 10px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--muted); }
  .field:focus-within { border-color: var(--accent); box-shadow: 0 0 0 3px var(--ring); }
  .field input { flex: 1; border: 0; outline: none; background: none; color: var(--text); font: inherit; font-size: 14px; }
  .srcs { display: inline-flex; padding: 3px; border-radius: 9px; background: var(--chip); }
  .srcs button { height: 32px; padding: 0 12px; border: 0; border-radius: 7px; background: none; color: var(--muted); font: inherit; font-size: 12.5px; font-weight: 700; cursor: pointer; }
  .srcs button.on { background: var(--surface); color: var(--accent); box-shadow: var(--shadow-sm); }

  .body { flex: 1; min-height: 0; display: flex; }
  .results { flex: 1; min-width: 0; overflow-y: auto; padding: 4px 10px 10px; }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .results li { display: flex; align-items: center; gap: 6px; padding: 4px 6px 4px 4px; border-radius: 11px; transition: background 0.1s; }
  .results li:hover { background: var(--chip); }
  .results li.sel { background: var(--accent-soft); box-shadow: inset 0 0 0 1.5px var(--accent); }
  .row { flex: 1; min-width: 0; display: flex; align-items: center; gap: 12px; padding: 4px; border: 0; background: none; color: inherit; font: inherit; text-align: left; cursor: pointer; }
  .thumb { flex: none; width: 40px; height: 58px; border-radius: 6px; overflow: hidden; display: grid; place-items: center; background: var(--chip); color: var(--muted); }
  .thumb img { width: 100%; height: 100%; object-fit: cover; }
  .rbody { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 5px; }
  .rtitle { font-size: 13px; font-weight: 550; line-height: 1.35; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .trusted { display: inline-block; width: 8px; height: 8px; border-radius: 50%; background: var(--ok); margin-right: 6px; vertical-align: middle; }
  .rmeta { display: flex; align-items: center; gap: 10px; font-size: 12px; color: var(--muted); font-variant-numeric: tabular-nums; flex-wrap: wrap; }
  .kb { font-size: 10.5px; font-weight: 700; padding: 1px 6px; border-radius: 5px; background: var(--chip); color: var(--text-2); }
  .kb.k-film { background: rgb(224 69 123 / 0.12); color: #e0457b; }
  .kb.k-series { background: rgb(59 130 246 / 0.12); color: #3b82f6; }
  .kb.k-anime { background: rgb(249 115 22 / 0.13); color: #f97316; }
  .kb.k-game { background: rgb(22 163 74 / 0.12); color: #16a34a; }
  .kb.k-software { background: rgb(8 145 178 / 0.12); color: #0891b2; }
  .kb.k-book { background: rgb(161 98 7 / 0.12); color: #a16207; }
  .kb.k-audio { background: rgb(147 51 234 / 0.12); color: #9333ea; }
  .lb { font-size: 10px; font-weight: 800; padding: 1px 5px; border-radius: 4px; border: 1px solid var(--line-strong); color: var(--text-2); }
  .seed { color: var(--ok); }
  .seed.dead { color: var(--err); }
  .dl { flex: none; display: grid; place-items: center; width: 34px; height: 34px; border-radius: 9px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--accent); cursor: pointer; }
  .dl:hover { background: var(--accent); border-color: var(--accent); color: white; }
  .done { flex: none; display: inline-flex; align-items: center; gap: 5px; color: var(--ok); font-size: 12.5px; font-weight: 700; padding: 0 9px; }
  .done.big { padding: 0 6px; }

  /* filtres */
  .filters { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding: 0 18px 10px; }
  .filters select { height: 30px; padding: 0 10px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: inherit; font-size: 12.5px; cursor: pointer; }
  .chips { display: inline-flex; padding: 2px; border-radius: 9px; background: var(--chip); }
  .chips button { display: inline-flex; align-items: center; gap: 5px; height: 26px; padding: 0 9px; border: 0; border-radius: 7px; background: none; color: var(--muted); font: inherit; font-size: 12px; font-weight: 650; cursor: pointer; }
  .chips button.on { background: var(--surface); color: var(--accent); box-shadow: var(--shadow-sm); }
  .chips em { font-style: normal; font-size: 10.5px; opacity: 0.7; }
  .team { font-size: 10.5px; font-weight: 800; padding: 1px 6px; border-radius: 5px; background: rgb(245 158 11 / 0.15); color: #d97706; }
  .alive.ep { margin-left: auto; color: var(--accent); font-weight: 650; }
  .alive.ep + .alive { margin-left: 0; }
  .alive { display: inline-flex; align-items: center; gap: 6px; font-size: 12.5px; color: var(--text-2); cursor: pointer; margin-left: auto; }

  /* squelette */
  .skel li { display: flex; align-items: center; gap: 12px; padding: 8px; }
  .sk-thumb { width: 40px; height: 58px; border-radius: 6px; }
  .sk-lines { flex: 1; display: flex; flex-direction: column; gap: 8px; }
  .sk-lines i { display: block; height: 11px; border-radius: 6px; }
  .sk-lines i:first-child { width: 72%; }
  .sk-lines i:last-child { width: 44%; }
  .skel.small .sk-lines i { width: 90%; }
  .sk-thumb, .sk-lines i { background: linear-gradient(90deg, var(--chip) 0%, var(--line-strong) 50%, var(--chip) 100%); background-size: 200% 100%; animation: shimmer 1.2s ease-in-out infinite; animation-delay: var(--d); }
  @keyframes shimmer { 0% { background-position: 100% 0; } 100% { background-position: -100% 0; } }

  /* aperçu */
  .preview { width: 360px; flex: none; display: flex; flex-direction: column; border-left: 1px solid var(--line); background: var(--bg); min-height: 0; }
  .pv-head { position: relative; display: flex; gap: 14px; padding: 16px 16px 10px; }
  .pv-poster { width: 86px; aspect-ratio: 2 / 3; object-fit: cover; border-radius: 8px; box-shadow: var(--shadow-sm); flex: none; }
  .pv-poster.ph { display: grid; place-items: center; background: var(--chip); color: var(--muted); }
  .pv-ht { min-width: 0; display: flex; flex-direction: column; gap: 6px; padding-right: 22px; }
  .pv-ht h3 { margin: 0; font-size: 16px; line-height: 1.25; }
  .pv-stats { display: flex; flex-wrap: wrap; gap: 4px 12px; font-size: 12px; color: var(--muted); }
  .pv-stats b { color: var(--text); }
  .pv-x { position: absolute; top: 12px; right: 12px; display: grid; place-items: center; width: 26px; height: 26px; border-radius: 7px; border: 0; background: var(--chip); color: var(--muted); cursor: pointer; }
  .pv-raw { margin: 0 16px 10px; font-size: 11px; color: var(--muted); font-family: ui-monospace, "Cascadia Code", monospace; word-break: break-all; line-height: 1.4; }
  .pv-files { flex: 1; min-height: 0; overflow-y: auto; padding: 0 12px; border-top: 1px solid var(--line); }
  .pv-files h4 { margin: 10px 4px 6px; font-size: 11.5px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
  .pv-files li { display: flex; gap: 8px; padding: 5px 4px; border-radius: 6px; font-size: 12px; }
  .pv-files li:hover { background: var(--chip); }
  .fname { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .fsize { color: var(--muted); white-space: nowrap; font-variant-numeric: tabular-nums; }
  .pv-actions { display: flex; justify-content: flex-end; align-items: center; gap: 8px; padding: 12px 14px; border-top: 1px solid var(--line); }

  .pad { flex: 1; padding: 16px 20px; display: flex; flex-direction: column; gap: 12px; }
  textarea { flex: 1; resize: none; padding: 12px; border-radius: 10px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: 13px ui-monospace, "Cascadia Code", monospace; outline: none; }
  textarea:focus { border-color: var(--accent); box-shadow: 0 0 0 3px var(--ring); }
  .actions { display: flex; justify-content: flex-end; }
  .drop { align-items: center; justify-content: center; text-align: center; color: var(--muted); }
  .drop p { margin: 0; color: var(--text); }
  .drop span { color: var(--muted); font-size: 12.5px; }

  .hint { color: var(--muted); font-size: 13px; padding: 20px 10px; margin: 0; display: flex; align-items: center; gap: 10px; }
  .hint.err { color: var(--err); }
  .spinner { width: 14px; height: 14px; border-radius: 50%; border: 2px solid var(--line-strong); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  footer { padding: 10px 20px; border-top: 1px solid var(--line); font-size: 12px; color: var(--muted); }
</style>