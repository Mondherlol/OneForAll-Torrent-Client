<script>
  import { fade, fly } from "svelte/transition";
  import { untrack } from "svelte";
  import { open as pickFiles } from "@tauri-apps/plugin-dialog";
  import Icon from "./Icon.svelte";
  import { nxbrewLinks, openUrl, previewTorrent, searchTorrents, stageTorrents } from "./api.js";
  import { KINDS, KIND_LABEL, fmtAgo, fmtSize } from "./model.js";
  import { arts, flash, openMenu, wantArt } from "./ui.svelte.js";
  import icoNyaa from "../assets/sources/nyaa.png";
  import icoOnlineFix from "../assets/sources/online-fix.ico";
  import icoFitgirl from "../assets/sources/fitgirl.ico";
  import icoNxbrew from "../assets/sources/nxbrew.png";

  let { torrents, onclose, onadded, initial = null } = $props();

  const TABS = [
    { id: "search", label: "Rechercher", icon: "search" },
    { id: "link", label: "Lien", icon: "link" },
    { id: "file", label: "Fichier .torrent", icon: "torrent" },
  ];
  let tab = $state("search");
  const LINK_HINT = [
    "Un lien par ligne :",
    "• magnet:?xt=urn:btih:…  ou  https://…/fichier.torrent → torrent",
    "• https://…/fichier.zip (lien direct) → onglet DL, avec reprise auto",
    "• lien YouTube, X, Insta, TikTok… → onglet DL, vidéo en meilleure qualité",
  ].join("\n");
  const SOURCES = {
    c411: { label: "C411", mono: "C", color: "#1f9d55", desc: "Films, séries, animés, jeux… en FR" },
    nyaa: { label: "Nyaa", ico: icoNyaa, desc: "Animés et mangas" },
    "online-fix": { label: "Online-Fix", ico: icoOnlineFix, desc: "Jeux PC jouables en ligne" },
    fitgirl: { label: "FitGirl", ico: icoFitgirl, desc: "Repacks de jeux PC" },
    nxbrew: { label: "NXBrew", ico: icoNxbrew, desc: "Jeux Switch (liens d'hébergeurs)" },
  };
  // Genre → sites où ça a du sens de chercher
  const GENRES = [
    { id: "all", label: "Tout", icon: "all", sources: ["c411", "nyaa", "online-fix", "fitgirl", "nxbrew"] },
    { id: "film", label: "Films", icon: "film", sources: ["c411"] },
    { id: "series", label: "Séries", icon: "series", sources: ["c411"] },
    { id: "anime", label: "Animés", icon: "anime", sources: ["c411", "nyaa"] },
    { id: "game", label: "Jeux", icon: "game", sources: ["online-fix", "fitgirl", "nxbrew", "c411"] },
    { id: "software", label: "Logiciels", icon: "software", sources: ["c411"] },
    { id: "audio", label: "Musique", icon: "audio", sources: ["c411"] },
    { id: "book", label: "Livres", icon: "book", sources: ["c411", "nyaa"] },
  ];
  let genre = $state(localStorage.getItem("oneforall.genre") ?? "all");
  if (!GENRES.some((g) => g.id === genre)) genre = "all";
  const genreSources = $derived(GENRES.find((g) => g.id === genre).sources);
  // "all" = tous les sites du genre, en parallèle
  let source = $state(localStorage.getItem("oneforall.source") ?? "all");
  $effect(() => localStorage.setItem("oneforall.source", source));
  $effect(() => localStorage.setItem("oneforall.genre", genre));
  const curGenre = $derived(GENRES.find((g) => g.id === genre));
  // NXBrew est lent : seulement quand on le choisit
  const targets = $derived(source === "all" || !genreSources.includes(source) ? genreSources.filter((x) => x !== "nxbrew") : [source]);
  function genreMenu(e) {
    const r = e.currentTarget.getBoundingClientRect();
    openMenu({ preventDefault() {}, stopPropagation() {}, clientX: r.left, clientY: r.bottom + 6 }, GENRES.map((g) => ({ label: g.label, icon: g.icon, checked: genre === g.id, run: () => pickGenre(g.id) })));
  }
  let query = $state("");
  let results = $state(null);
  let status = $state({}); // site → { loading } | { n } | { error }
  const loading = $derived(Object.values(status).some((st) => st.loading));
  const failed = $derived(Object.entries(status).filter(([, st]) => st.error));
  const error = $derived(results && !results.length && !loading && failed.length && failed.length === Object.keys(status).length ? failed.map(([id, st]) => (failed.length > 1 ? `${SOURCES[id].label} : ${st.error}` : st.error)).join(" · ") : null);
  let srcF = $state("all");

  // Recherches récentes
  let recent = $state(JSON.parse(localStorage.getItem("oneforall.recent") ?? "[]"));
  function remember(q) {
    recent = [q, ...recent.filter((x) => x.toLowerCase() !== q.toLowerCase())].slice(0, 10);
    localStorage.setItem("oneforall.recent", JSON.stringify(recent));
  }
  function forgetRecent() {
    recent = [];
    localStorage.removeItem("oneforall.recent");
  }
  function pickGenre(id) {
    genre = id;
    const srcs = GENRES.find((g) => g.id === id).sources;
    if (source !== "all" && !srcs.includes(source)) source = "all";
    if (results) search();
  }
  function pickSource(id) {
    source = id;
    if (results) search();
  }
  let links = $state("");
  let added = $state(new Set());
  let inputEl = $state();

  $effect(() => {
    tab;
    setTimeout(() => inputEl?.focus(), 50);
  });

  const have = $derived(new Set(torrents.map((t) => t.hash.toLowerCase())));

  let run = 0;
  async function search() {
    const q = query.trim();
    if (!q) return;
    const id = ++run;
    const srcs = targets;
    remember(q);
    selected = null;
    kindF = "all";
    srcF = "all";
    results = [];
    status = Object.fromEntries(srcs.map((x) => [x, { loading: true }]));
    // Chaque site répond à son rythme : les résultats arrivent au fur et à mesure
    await Promise.all(
      srcs.map(async (x) => {
        try {
          const res = await searchTorrents(q, x);
          if (id !== run) return;
          const seen = new Set(results.map((r) => r.url));
          results = [...results, ...res.filter((r) => !seen.has(r.url))];
          status[x] = { n: res.length };
        } catch (e) {
          if (id === run) status[x] = { error: String(e) };
        }
      }),
    );
    if (id === run && genre !== "all" && results.some((r) => r.kind === genre)) kindF = genre;
  }

  // Les torrents sont ajoutés sans démarrer ; l'app propose ensuite de choisir les fichiers (onadded)
  async function addOne(r) {
    if (r.source === "nxbrew") return select(r);
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
    if (!urls.length) return flash("Colle au moins un lien magnet:… ou https://…");
    try {
      const res = await stageTorrents(urls);
      onadded(res);
      if (res.hashes.length || res.direct?.length) onclose();
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
  const srcsIn = $derived.by(() => {
    const counts = {};
    for (const r of results ?? []) counts[r.source] = (counts[r.source] ?? 0) + 1;
    return Object.keys(SOURCES).filter((id) => counts[id]).map((id) => ({ id, n: counts[id] }));
  });
  const hasNyaa = $derived(srcsIn.some((x) => x.id === "nyaa"));
  const collator = new Intl.Collator("fr", { numeric: true, sensitivity: "base" });
  const shown = $derived.by(() => {
    let list = (results ?? []).filter(
      (r) =>
        (kindF === "all" || r.kind === kindF) &&
        (srcF === "all" || r.source === srcF) &&
        (r.source !== "nyaa" || langF === "all" || r.lang === langF) &&
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
      if (init.source) {
        if (!GENRES.find((g) => g.id === genre).sources.includes(init.source)) genre = "all";
        source = init.source;
      } else {
        genre = GENRES.some((g) => g.id === init.kind) ? init.kind : "all";
        source = "all";
      }
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
      if (r.source === "nxbrew") {
        // Pas de .torrent : liens d'hébergeurs (via ouo.io) par partie du jeu
        const nx = await nxbrewLinks(r.page);
        if (selected?.url === url) preview = { nx };
        return;
      }
      if (url.startsWith("magnet:")) {
        preview = { error: "La liste des fichiers s'affiche après l'ajout (lien magnet) : tu pourras choisir quoi télécharger." };
        return;
      }
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
    {#snippet sico(id, big = false)}
      {#if SOURCES[id].ico}<img class={big ? "src-ico" : "mini-ico"} src={SOURCES[id].ico} alt="" />{:else}<span class={big ? "src-ico mono" : "mini-ico mono"} style="background:{SOURCES[id].color}">{SOURCES[id].mono}</span>{/if}
    {/snippet}
    <div class="hero">
      <form class="bar" onsubmit={(e) => (e.preventDefault(), search())}>
        <div class="field">
          <button type="button" class="gpick k-{genre}" title="Genre : choisit les sites où chercher" onclick={genreMenu}>
            <Icon name={curGenre.icon} size={15} />{curGenre.label}<Icon name="chevDown" size={13} />
          </button>
          <span class="fsep"></span>
          <input bind:this={inputEl} bind:value={query} placeholder={genre === "game" ? "Ex. : Elden Ring" : genre === "anime" ? "Ex. : Frieren 1080p" : "Ex. : Black Clover 1080p"} spellcheck="false" />
          {#if query}<button type="button" class="clear" title="Effacer" onclick={() => ((query = ""), inputEl?.focus())}><Icon name="close" size={13} /></button>{/if}
          <button class="go" disabled={!query.trim()}>{#if loading}<span class="spinner light"></span>{:else}<Icon name="search" size={15} />{/if}Rechercher</button>
        </div>
      </form>

      <div class="srcs" aria-label="Sites">
        {#if genreSources.length > 1}
          <button type="button" class="src" class:on={source === "all" || !genreSources.includes(source)} onclick={() => pickSource("all")} title="Chercher sur tous ces sites en même temps (sauf NXBrew, trop lent : clique dessus pour y chercher)">Partout</button>
        {/if}
        {#each genreSources as id}
          {@const st = status[id]}
          <button type="button" class="src" class:on={source === id || genreSources.length === 1} class:off={!targets.includes(id)} onclick={() => pickSource(id)} title={st?.error ?? SOURCES[id].desc}>
            {@render sico(id, true)}{SOURCES[id].label}
            {#if st?.loading}<span class="spinner"></span>
            {:else if st?.error}<span class="src-n err">!</span>
            {:else if st}<span class="src-n">{st.n}</span>{/if}
          </button>
        {/each}
      </div>
    </div>

    {#if results?.length}
      <!-- Tris et filtres -->
      <div class="filters">
        <select bind:value={sortBy} title="Trier">
          {#each SORTS as s}<option value={s.id}>{s.label}</option>{/each}
        </select>
        {#if kindsIn.length > 1}
          <select bind:value={kindF} title="Type">
            <option value="all">Tous types ({results.length})</option>
            {#each kindsIn as k}<option value={k.id}>{k.label} ({k.n})</option>{/each}
          </select>
        {/if}
        {#if srcsIn.length > 1}
          <select bind:value={srcF} title="Site">
            <option value="all">Tous sites</option>
            {#each srcsIn as x}<option value={x.id}>{SOURCES[x.id].label} ({x.n})</option>{/each}
          </select>
        {/if}
        {#if hasNyaa}
          <select bind:value={langF} title="Langue (Nyaa)">
            {#each LANGS as l}<option value={l.id}>{l.label}</option>{/each}
          </select>
        {/if}
        {#if loading}<span class="pending"><span class="spinner"></span>{Object.entries(status).filter(([, st]) => st.loading).map(([id]) => SOURCES[id].label).join(", ")}…</span>{/if}
        {#each failed as [id, st]}<span class="fail" title={st.error}>{SOURCES[id].label} n'a pas répondu</span>{/each}
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
        {:else if loading && !results?.length}
          <!-- Squelette pendant la recherche -->
          <ul class="skel">
            {#each Array(7) as _, i}
              <li style="--d:{i * 70}ms"><span class="sk-thumb"></span><span class="sk-lines"><i></i><i></i></span></li>
            {/each}
          </ul>
        {:else if results && !shown.length}
          <div class="empty">
            <Icon name="search" size={34} />
            <b>{results.length ? "Aucun résultat avec ces filtres" : `Aucun résultat pour « ${query} »`}</b>
            {#if !results.length && genre !== "all"}<button class="btn" onclick={() => pickGenre("all")}><Icon name="layers" size={14} />Chercher sur tous les sites</button>{/if}
          </div>
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
                      {#if SOURCES[r.source]}<span class="rsrc" title={SOURCES[r.source].label}>{@render sico(r.source)}</span>{/if}
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
          <div class="welcome">
            <span class="w-ic"><Icon name={GENRES.find((g) => g.id === genre).icon} size={30} /></span>
            <h3>Que veux-tu télécharger ?</h3>
            <p class="w-where">
              Recherche sur
              {#each targets as id, i}<span class="w-src">{@render sico(id)}{SOURCES[id].label}</span>{i < targets.length - 2 ? ", " : i === targets.length - 2 ? " et " : ""}{/each}
            </p>
            {#if recent.length}
              <div class="recent">
                <span class="r-head">Récemment <button class="r-clear" onclick={forgetRecent}>Effacer</button></span>
                <div class="r-list">
                  {#each recent as q}<button onclick={() => ((query = q), search())}><Icon name="search" size={12} />{q}</button>{/each}
                </div>
              </div>
            {/if}
            <p class="w-tip">Clique sur un résultat pour voir ses fichiers avant de le télécharger.</p>
          </div>
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
            {:else if preview?.nx}
              {#if !preview.nx.length}
                <p class="hint">Aucun lien trouvé sur la page — ouvre-la sur le site.</p>
              {:else}
                <p class="nx-note">Les liens passent par <b>ouo.io</b> (vérification à faire dans le navigateur), puis l'hébergeur. Si tu obtiens un lien direct, colle-le dans <b>+ → Lien</b> : il ira dans l'onglet Téléchargements, avec reprise auto.</p>
                {#each preview.nx as s}
                  <h4>{s.label}</h4>
                  {#each s.hosts as h}
                    <div class="nx-host">
                      <span class="nx-name">{h.host}</span>
                      <span class="nx-links">
                        {#each h.links as l}<button class="nx-link" title={l.url} onclick={() => openUrl(l.url)}>{l.label}</button>{/each}
                      </span>
                    </div>
                  {/each}
                {/each}
              {/if}
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
            {:else if selected.source !== "nxbrew"}
              <button class="btn primary" onclick={() => addOne(selected)}><Icon name="down" size={14} />Télécharger</button>
            {/if}
          </div>
        </aside>
      {/if}
    </div>
  {:else if tab === "link"}
    <div class="pad">
      <textarea bind:this={inputEl} bind:value={links} placeholder={LINK_HINT} spellcheck="false"></textarea>
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

  .hero { display: flex; flex-direction: column; gap: 10px; padding: 14px 18px 10px; }
  .bar { display: flex; }
  .field { flex: 1; display: flex; align-items: center; gap: 8px; height: 46px; padding: 0 5px; border-radius: 12px; border: 1.5px solid var(--line-strong); background: var(--surface); transition: border-color 0.15s, box-shadow 0.15s; }
  .field:focus-within { border-color: var(--accent); box-shadow: 0 0 0 4px var(--ring); }
  .field input { flex: 1; min-width: 0; border: 0; outline: none; background: none; color: var(--text); font: inherit; font-size: 15px; padding-left: 4px; }
  .gpick { --c: var(--accent); display: inline-flex; align-items: center; gap: 7px; height: 36px; padding: 0 10px; border: 0; border-radius: 8px; background: none; color: var(--c); font: inherit; font-size: 13px; font-weight: 700; cursor: pointer; }
  .gpick:hover { background: var(--chip); }
  .gpick.k-film { --c: #e0457b; }
  .gpick.k-series { --c: #3b82f6; }
  .gpick.k-anime { --c: #f97316; }
  .gpick.k-game { --c: #16a34a; }
  .gpick.k-software { --c: #0891b2; }
  .gpick.k-book { --c: #a16207; }
  .gpick.k-audio { --c: #9333ea; }
  .fsep { width: 1px; height: 22px; background: var(--line-strong); }
  .clear { display: grid; place-items: center; width: 24px; height: 24px; border-radius: 50%; border: 0; background: var(--chip); color: var(--muted); cursor: pointer; }
  .clear:hover { color: var(--text); }
  .go { display: inline-flex; align-items: center; gap: 7px; height: 36px; padding: 0 16px; border: 0; border-radius: 8px; background: var(--accent); color: white; font: inherit; font-size: 13px; font-weight: 650; cursor: pointer; transition: filter 0.12s; }
  .go:hover:not(:disabled) { filter: brightness(1.08); }
  .go:disabled { opacity: 0.45; cursor: default; }

  .srcs { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; }
  .src { display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 10px; border-radius: 999px; border: 0; background: none; color: var(--muted); font: inherit; font-size: 12.5px; font-weight: 600; cursor: pointer; transition: background 0.12s, color 0.12s; }
  .src:hover { background: var(--chip); color: var(--text); }
  .src.on { background: var(--accent-soft); color: var(--accent); }
  .src.off:not(.on) { opacity: 0.55; }
  .src-ico { width: 16px; height: 16px; border-radius: 4px; object-fit: contain; flex: none; }
  .src-n { min-width: 18px; height: 17px; padding: 0 5px; border-radius: 9px; background: var(--chip); color: var(--text-2); font-size: 10.5px; font-weight: 800; display: grid; place-items: center; font-variant-numeric: tabular-nums; }
  .src-n.err { background: var(--err-soft); color: var(--err); }
  .mini-ico { width: 14px; height: 14px; border-radius: 3px; object-fit: contain; flex: none; vertical-align: -2px; }
  .mono { display: inline-grid; place-items: center; color: white; font-weight: 800; font-size: 10px; line-height: 1; }
  .mini-ico.mono { font-size: 9px; }
  .rsrc { display: inline-flex; }
  .pending { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; color: var(--muted); }
  .fail { font-size: 12px; color: var(--err); }

  .welcome { display: flex; flex-direction: column; align-items: center; text-align: center; gap: 8px; padding: 7vh 20px 20px; color: var(--muted); }
  .w-ic { display: grid; place-items: center; width: 64px; height: 64px; border-radius: 20px; background: var(--accent-soft); color: var(--accent); margin-bottom: 4px; }
  .welcome h3 { margin: 0; font-size: 18px; color: var(--text); }
  .w-where { margin: 0; font-size: 13px; line-height: 1.8; }
  .w-src { display: inline-flex; align-items: center; gap: 5px; font-weight: 650; color: var(--text-2); }
  .w-tip { margin: 14px 0 0; font-size: 12px; }
  .recent { margin-top: 14px; display: flex; flex-direction: column; align-items: center; gap: 8px; max-width: 620px; }
  .r-head { font-size: 11px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.06em; }
  .r-clear { margin-left: 6px; border: 0; background: none; color: var(--accent); font: inherit; font-size: 11px; text-transform: none; letter-spacing: 0; cursor: pointer; }
  .r-list { display: flex; flex-wrap: wrap; justify-content: center; gap: 6px; }
  .r-list button { display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 11px; border-radius: 999px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text-2); font: inherit; font-size: 12.5px; cursor: pointer; }
  .r-list button:hover { border-color: var(--accent); color: var(--accent); }
  .empty { display: flex; flex-direction: column; align-items: center; gap: 10px; padding: 8vh 20px; color: var(--muted); text-align: center; }
  .empty b { color: var(--text); font-size: 14px; }

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
  .filters { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding: 2px 18px 10px; border-bottom: 1px solid var(--line); margin-bottom: 4px; }
  .filters select { height: 28px; padding: 0 8px; border-radius: 7px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: inherit; font-size: 12px; cursor: pointer; }
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
  .nx-note { margin: 10px 4px 4px; font-size: 11.5px; line-height: 1.45; color: var(--muted); }
  .nx-host { display: flex; align-items: center; gap: 8px; padding: 5px 4px; border-radius: 6px; }
  .nx-host:hover { background: var(--chip); }
  .nx-name { width: 84px; flex: none; font-size: 12px; font-weight: 700; }
  .nx-links { display: flex; flex-wrap: wrap; gap: 4px; }
  .nx-link { height: 24px; padding: 0 8px; border-radius: 6px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--accent); font: inherit; font-size: 11.5px; font-weight: 650; cursor: pointer; }
  .nx-link:hover { border-color: var(--accent); background: var(--accent-soft); }
  .pv-actions { display: flex; justify-content: flex-end; align-items: center; gap: 8px; padding: 12px 14px; border-top: 1px solid var(--line); }

  .pad { flex: 1; padding: 16px 20px; display: flex; flex-direction: column; gap: 12px; }
  textarea { flex: 1; resize: none; padding: 12px; border-radius: 10px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: 13px ui-monospace, "Cascadia Code", monospace; outline: none; }
  textarea:focus { border-color: var(--accent); box-shadow: 0 0 0 3px var(--ring); }
  .actions { display: flex; justify-content: flex-end; }
  .drop { align-items: center; justify-content: center; text-align: center; color: var(--muted); }
  .drop p { margin: 0; color: var(--text); }
  .drop span { color: var(--muted); font-size: 12.5px; }

  .hint { color: var(--muted); font-size: 13px; padding: 20px 10px; margin: 0; display: flex; align-items: center; gap: 10px; white-space: pre-line; }
  .hint.err { color: var(--err); }
  .spinner.light { border-color: rgb(255 255 255 / 0.35); border-top-color: white; }
  .spinner { flex: none; width: 14px; height: 14px; border-radius: 50%; border: 2px solid var(--line-strong); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  footer { padding: 10px 20px; border-top: 1px solid var(--line); font-size: 12px; color: var(--muted); }
</style>