<script>
  // Fenêtre « Choisir l'image » : propositions des sources (IGDB, TMDB, AniList…) selon le type,
  // ou une image à soi (fichier, glisser-déposer, Ctrl+V d'une image ou d'un lien).
  import { fade, fly } from "svelte/transition";
  import { open as pickFile } from "@tauri-apps/plugin-dialog";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import Icon from "./Icon.svelte";
  import { artChoices, setArtwork } from "./api.js";
  import { arts, flash, refreshArt, ui } from "./ui.svelte.js";

  const job = $derived(ui.artPick); // { key, kind, title, year }
  const current = $derived(arts[job.key]);

  const KINDS = [
    { id: "film", label: "Film" },
    { id: "series", label: "Série" },
    { id: "anime", label: "Animé" },
    { id: "game", label: "Jeu" },
    { id: "book", label: "Livre / manga" },
    { id: "audio", label: "Audio" },
    { id: "software", label: "Logiciel" },
  ];
  let query = $state(ui.artPick.title);
  let kind = $state(KINDS.some((k) => k.id === ui.artPick.kind) ? ui.artPick.kind : "film");
  let list = $state(null); // null = recherche en cours
  let src = $state("all");
  let broken = $state(new Set());
  let busy = $state(null); // url en cours d'application
  let dropping = $state(false);
  let run = 0;

  async function search() {
    const id = ++run;
    list = null;
    src = "all";
    broken = new Set();
    try {
      const r = await artChoices(kind, query.trim() || job.title, job.year);
      if (id === run) list = r;
    } catch (e) {
      if (id === run) list = [];
      flash(String(e));
    }
  }
  search();

  const sources = $derived.by(() => {
    const n = {};
    for (const c of list ?? []) if (!broken.has(c.url)) n[c.source] = (n[c.source] ?? 0) + 1;
    return Object.entries(n);
  });
  const shown = $derived((list ?? []).filter((c) => !broken.has(c.url) && (src === "all" || c.source === src)));

  const close = () => (ui.artPick = null);

  async function apply(args, label) {
    busy = args.url ?? "custom";
    try {
      const art = await setArtwork(job.key, args);
      arts[job.key] = art;
      flash(label);
      close();
    } catch (e) {
      flash(String(e));
    } finally {
      busy = null;
    }
  }
  const choose = (c) => apply({ url: c.url, fallback: c.fallback }, "Image changée");

  async function importFile() {
    const path = await pickFile({ multiple: false, filters: [{ name: "Images", extensions: ["jpg", "jpeg", "png", "webp"] }] });
    if (path) apply({ path }, "Image importée");
  }

  async function automatic() {
    await refreshArt(job.key);
    flash("Image automatique rétablie");
    close();
  }

  // Ctrl+V : une image copiée, ou le lien d'une image
  function onpaste(e) {
    if (e.target instanceof HTMLInputElement) return;
    const file = [...(e.clipboardData?.items ?? [])].find((i) => i.type.startsWith("image/"))?.getAsFile();
    if (file) {
      e.preventDefault();
      const r = new FileReader();
      r.onload = () => apply({ data: r.result }, "Image collée");
      r.readAsDataURL(file);
      return;
    }
    const text = e.clipboardData?.getData("text")?.trim();
    if (text && /^https?:\/\/\S+$/i.test(text)) {
      e.preventDefault();
      apply({ url: text }, "Image changée");
    }
  }

  // Glisser-déposer d'une image (la fenêtre reçoit les chemins des fichiers)
  const IMG = /\.(jpe?g|png|webp)$/i;
  $effect(() => {
    let un;
    getCurrentWebview()
      .onDragDropEvent((ev) => {
        const p = ev.payload;
        if (p.type === "enter" || p.type === "over") dropping = (p.paths ?? []).some((x) => IMG.test(x)) || dropping;
        else if (p.type === "leave") dropping = false;
        else if (p.type === "drop") {
          dropping = false;
          const path = p.paths.find((x) => IMG.test(x));
          if (path) apply({ path }, "Image importée");
        }
      })
      .then((f) => (un = f));
    return () => un?.();
  });

  function imgError(c, e) {
    if (c.fallback && e.currentTarget.src !== c.fallback) e.currentTarget.src = c.fallback;
    else broken = new Set([...broken, c.url]);
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && close()} {onpaste} />

<div class="backdrop" transition:fade={{ duration: 140 }} onclick={close} role="presentation"></div>
<div class="modal" class:dropping transition:fly={{ y: 16, duration: 200 }} role="dialog" aria-label="Choisir l'image">
  <header>
    <span class="cur">
      {#if current?.url}<img src={current.url} alt="" />{:else}<Icon name={job.kind} size={20} />{/if}
    </span>
    <div class="ht">
      <small>Choisir l'image</small>
      <h2 title={job.title}>{job.title}</h2>
    </div>
    <button class="x" title="Fermer (Échap)" onclick={close}><Icon name="close" size={15} /></button>
  </header>

  <form class="bar" onsubmit={(e) => (e.preventDefault(), search())}>
    <div class="field">
      <Icon name="search" size={15} />
      <input bind:value={query} placeholder="Titre à chercher" spellcheck="false" />
    </div>
    <select bind:value={kind} onchange={search} title="Type (change les sources)">
      {#each KINDS as k}<option value={k.id}>{k.label}</option>{/each}
    </select>
    <button class="btn primary" type="submit">Chercher</button>
  </form>

  {#if sources.length > 1}
    <div class="pills">
      <button class:on={src === "all"} onclick={() => (src = "all")}>Toutes <i>{sources.reduce((t, [, n]) => t + n, 0)}</i></button>
      {#each sources as [s, n]}
        <button class:on={src === s} onclick={() => (src = s)}>{s} <i>{n}</i></button>
      {/each}
    </div>
  {/if}

  <div class="grid">
    <!-- Image à soi -->
    <button class="tile own" onclick={importFile} title="Importer une image (ou glisse-la ici, ou Ctrl+V)">
      <span class="own-ic"><Icon name="image" size={22} /></span>
      <b>Importer</b>
      <small>fichier, glisser-déposer<br />ou Ctrl+V</small>
    </button>

    {#if list === null}
      {#each Array(9) as _}<span class="tile sk"></span>{/each}
    {:else}
      {#each shown as c (c.url)}
        <button class="tile" class:sel={current?.url === c.url} class:busy={busy === c.url} disabled={!!busy} onclick={() => choose(c)} title={`${c.title}${c.sub ? ` (${c.sub})` : ""} · ${c.source}`}>
          <img src={c.url} alt="" loading="lazy" onerror={(e) => imgError(c, e)} />
          <span class="src">{c.source}</span>
          {#if current?.url === c.url}<span class="check"><Icon name="check" size={13} /></span>{/if}
          <span class="cap"><b>{c.title}</b>{#if c.sub}<small>{c.sub}</small>{/if}</span>
        </button>
      {/each}
    {/if}
  </div>
  {#if list && !shown.length}
    <p class="empty">Aucune image trouvée. Essaie un autre titre (en anglais, par exemple) ou un autre type.</p>
  {/if}

  <footer>
    <span class="hint"><Icon name="alert" size={13} />Colle une image ou un lien d'image (Ctrl+V) pour l'utiliser directement.</span>
    <button class="btn" onclick={automatic} title="Oublier le choix et retrouver l'image automatiquement"><Icon name="refresh" size={13} />Automatique</button>
  </footer>

  {#if dropping}
    <div class="drop" transition:fade={{ duration: 100 }}><Icon name="image" size={30} /><b>Dépose l'image ici</b></div>
  {/if}
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 150; background: var(--scrim); backdrop-filter: blur(3px); }
  .modal { position: fixed; z-index: 151; left: 50%; top: 50%; translate: -50% -50%; width: min(860px, 94vw); height: min(680px, 90vh); padding: 20px 22px 16px; border-radius: 18px; background: var(--bg-2); border: 1px solid var(--line-strong); box-shadow: var(--shadow-lg); display: flex; flex-direction: column; gap: 12px; }
  header { display: flex; align-items: center; gap: 13px; }
  .cur { flex: none; display: grid; place-items: center; width: 42px; aspect-ratio: 2 / 3; border-radius: 7px; overflow: hidden; background: var(--chip); color: var(--muted); box-shadow: 0 4px 12px rgb(0 0 0 / 0.18); }
  .cur img { width: 100%; height: 100%; object-fit: cover; }
  .ht { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .ht small { font-size: 12px; font-weight: 650; color: var(--accent); }
  h2 { margin: 0; font-size: 18px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .x { flex: none; align-self: flex-start; display: grid; place-items: center; width: 28px; height: 28px; border-radius: 8px; border: 0; background: var(--chip); color: var(--muted); cursor: pointer; }
  .x:hover { color: var(--text); }

  .bar { display: flex; gap: 8px; }
  .field { flex: 1; display: flex; align-items: center; gap: 8px; height: 38px; padding: 0 12px; border-radius: 10px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--muted); }
  .field:focus-within { border-color: var(--accent); }
  .field input { flex: 1; min-width: 0; border: 0; outline: 0; background: none; color: var(--text); font: inherit; font-size: 14px; }
  .bar select { height: 38px; padding: 0 10px; border-radius: 10px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: inherit; font-size: 13px; }
  .bar .btn { height: 38px; padding: 0 16px; }

  .pills { display: flex; gap: 6px; flex-wrap: wrap; }
  .pills button { display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 11px; border-radius: 999px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text-2); font: inherit; font-size: 12.5px; font-weight: 600; cursor: pointer; }
  .pills button:hover { border-color: var(--accent); }
  .pills button.on { background: var(--accent-soft); border-color: transparent; color: var(--accent); }
  .pills i { font-style: normal; font-size: 11px; color: var(--muted); }

  .grid { flex: 1; min-height: 0; overflow-y: auto; display: grid; grid-template-columns: repeat(auto-fill, minmax(118px, 1fr)); gap: 12px; align-content: start; padding: 2px 4px 6px 2px; }
  .tile { position: relative; aspect-ratio: 2 / 3; padding: 0; border-radius: 11px; overflow: hidden; border: 2px solid transparent; background: var(--chip); cursor: pointer; transition: transform 0.15s, border-color 0.15s, box-shadow 0.15s; }
  .tile:hover:not(:disabled) { transform: translateY(-2px); box-shadow: 0 10px 22px rgb(0 0 0 / 0.22); border-color: var(--accent); }
  .tile.sel { border-color: var(--accent); }
  .tile.busy { opacity: 0.6; }
  .tile img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .src { position: absolute; top: 6px; left: 6px; padding: 1px 6px; border-radius: 5px; font-size: 10px; font-weight: 700; background: rgb(0 0 0 / 0.6); color: white; backdrop-filter: blur(4px); }
  .check { position: absolute; top: 6px; right: 6px; display: grid; place-items: center; width: 20px; height: 20px; border-radius: 50%; background: var(--accent); color: white; }
  .cap { position: absolute; inset: auto 0 0 0; display: flex; flex-direction: column; gap: 1px; padding: 22px 8px 7px; text-align: left; background: linear-gradient(transparent, rgb(0 0 0 / 0.85)); color: white; opacity: 0; transition: opacity 0.15s; }
  .tile:hover .cap { opacity: 1; }
  .cap b { font-size: 11.5px; line-height: 1.25; overflow: hidden; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; }
  .cap small { font-size: 10.5px; opacity: 0.8; }
  .own { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 6px; border: 2px dashed var(--line-strong); background: var(--surface); color: var(--text-2); font: inherit; }
  .own:hover:not(:disabled) { color: var(--accent); }
  .own-ic { display: grid; place-items: center; width: 44px; height: 44px; border-radius: 12px; background: var(--accent-soft); color: var(--accent); }
  .own b { font-size: 13px; }
  .own small { font-size: 10.5px; line-height: 1.35; color: var(--muted); text-align: center; }
  .sk { cursor: default; background: linear-gradient(100deg, var(--chip) 30%, var(--line) 50%, var(--chip) 70%); background-size: 300% 100%; animation: shimmer 1.2s linear infinite; }
  @keyframes shimmer { from { background-position: 100% 0; } to { background-position: 0 0; } }
  .empty { margin: 0; font-size: 13px; color: var(--muted); text-align: center; }

  footer { display: flex; align-items: center; gap: 10px; padding-top: 10px; border-top: 1px solid var(--line); }
  .hint { flex: 1; display: flex; align-items: center; gap: 6px; font-size: 12px; color: var(--muted); }
  footer .btn { display: inline-flex; align-items: center; gap: 6px; }

  .drop { position: absolute; inset: 0; border-radius: 18px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; background: color-mix(in srgb, var(--bg-2) 88%, transparent); border: 2px dashed var(--accent); color: var(--accent); font-size: 15px; pointer-events: none; }
</style>
