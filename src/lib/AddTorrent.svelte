<script>
  import { fade, fly } from "svelte/transition";
  import { open as pickFiles } from "@tauri-apps/plugin-dialog";
  import Icon from "./Icon.svelte";
  import { searchTorrents, stageTorrents } from "./api.js";
  import { fmtAgo, fmtSize } from "./model.js";
  import { flash } from "./ui.svelte.js";

  let { torrents, onclose, onadded } = $props();

  const TABS = [
    { id: "search", label: "Rechercher", icon: "search" },
    { id: "link", label: "Lien magnet", icon: "link" },
    { id: "file", label: "Fichier .torrent", icon: "torrent" },
  ];
  let tab = $state("search");
  const SOURCES = [
    { id: "c411", label: "C411" },
    { id: "nyaa", label: "Nyaa" },
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

    <div class="results">
      {#if error}
        <p class="hint err">{error}</p>
      {:else if loading}
        <p class="hint"><span class="spinner"></span>Recherche sur {source === "c411" ? "C411" : "nyaa.si"}…</p>
      {:else if results && !results.length}
        <p class="hint">Aucun résultat pour « {query} ».</p>
      {:else if results}
        <ul>
          {#each results as r (r.url)}
            {@const already = have.has(r.hash) || added.has(r.url)}
            <li>
              <div class="rbody">
                <span class="rtitle" title={r.title}>{#if r.trusted}<i class="trusted" title="Uploader de confiance"></i>{/if}{r.title}</span>
                <span class="rmeta">
                  <b class="seed" class:dead={r.seeders === 0}>↑ {r.seeders}</b>
                  <span>↓ {r.leechers}</span>
                  <span>{fmtSize(r.size)}</span>
                  <span>{fmtAgo(ts(r.date))}</span>
                  <span class="cat">{r.category}</span>
                </span>
              </div>
              {#if already}
                <span class="done"><Icon name="check" size={14} />Ajouté</span>
              {:else}
                <button class="btn" onclick={() => addOne(r)}><Icon name="down" size={14} />Télécharger</button>
              {/if}
            </li>
          {/each}
        </ul>
      {:else}
        <p class="hint">Tape un titre puis Entrée. Les résultats sont triés par nombre de sources (seeders) : plus il y en a, plus c'est rapide.</p>
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
    width: min(860px, 94vw);
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

  .results { flex: 1; min-height: 0; overflow-y: auto; padding: 0 10px 10px; }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  li { display: flex; align-items: center; gap: 12px; padding: 9px 10px; border-radius: 10px; }
  li:hover { background: var(--chip); }
  .rbody { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .rtitle { font-size: 13px; font-weight: 550; line-height: 1.35; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .trusted { display: inline-block; width: 8px; height: 8px; border-radius: 50%; background: var(--ok); margin-right: 6px; vertical-align: middle; }
  .rmeta { display: flex; gap: 12px; font-size: 12px; color: var(--muted); font-variant-numeric: tabular-nums; flex-wrap: wrap; }
  .seed { color: var(--ok); }
  .seed.dead { color: var(--err); }
  .cat { opacity: 0.8; }
  .done { display: inline-flex; align-items: center; gap: 5px; color: var(--ok); font-size: 12.5px; font-weight: 700; padding: 0 10px; }

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
