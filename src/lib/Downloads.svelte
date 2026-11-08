<script>
  import { fly } from "svelte/transition";
  import { open as pickFolder } from "@tauri-apps/plugin-dialog";
  import Icon from "./Icon.svelte";
  import { addDownloads, downloadAction, moveDownloads, openPath, reveal } from "./api.js";
  import { KIND_LABEL, fmtAgo, fmtEta, fmtSize, fmtSpeed } from "./model.js";
  import { arts, confirmAsk, flash, openMenu, wantArt } from "./ui.svelte.js";

  /** downloads : liste de get_downloads ; onrefresh : recharge la liste tout de suite */
  let { downloads, onrefresh } = $props();

  let links = $state("");
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
    for (const d of downloads) wantArt({ artKey: d.art_key, kind: d.kind, title: d.title, year: d.year });
  });

  async function add() {
    const urls = links.split(/\s+/).filter((l) => /^https?:\/\//i.test(l));
    if (!urls.length) return flash("Colle au moins un lien https://…");
    try {
      const ids = await addDownloads(urls);
      links = "";
      flash(ids.length > 1 ? `${ids.length} téléchargements ajoutés` : "Téléchargement ajouté");
      onrefresh();
    } catch (e) {
      flash(String(e));
    }
  }

  async function act(items, action) {
    try {
      await downloadAction(items.map((d) => d.id), action);
      onrefresh();
    } catch (e) {
      flash(String(e));
    }
  }

  async function remove(d, withFile) {
    const what = `« ${d.name} »`;
    const ok = await confirmAsk(
      withFile ? `Supprimer ${what} et le fichier ?` : `Retirer ${what} de la liste ?`,
      withFile ? `${fmtSize(d.done)} seront effacés (corbeille).` : d.st === "done" ? "Le fichier reste sur le disque." : "Le fichier partiel (.part) est effacé.",
      withFile ? "Supprimer" : "Retirer",
      withFile,
    );
    if (ok) act([d], withFile ? "delete_files" : "delete");
  }

  async function moveTo(d) {
    const dir = await pickFolder({ directory: true, title: `Déplacer « ${d.name} » vers…`, defaultPath: d.dir || undefined });
    if (typeof dir !== "string") return;
    try {
      await moveDownloads([d.id], dir);
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

  const copy = (t) => navigator.clipboard.writeText(t).then(() => flash("Lien copié"));
  const openItem = (d) => (d.st === "done" ? openPath(d.path) : reveal(d.path, d.dir)).catch((e) => flash(String(e)));

  function menu(e, d) {
    openMenu(e, [
      d.st === "done" && { label: "Ouvrir", icon: "open", run: () => openPath(d.path).catch((err) => flash(String(err))) },
      { label: "Ouvrir le dossier", icon: "folder", run: () => reveal(d.path, d.dir).catch((err) => flash(String(err))) },
      "-",
      (d.st === "running" || d.st === "queued") && { label: "Pause", icon: "pause", run: () => act([d], "pause") },
      d.st === "paused" && { label: "Reprendre", icon: "play", run: () => act([d], "resume") },
      d.st === "error" && { label: "Réessayer", icon: "refresh", run: () => act([d], "resume") },
      { label: "Déplacer…", icon: "move", run: () => moveTo(d) },
      { label: "Copier le lien", icon: "copy", run: () => copy(d.url) },
      "-",
      { label: "Retirer de la liste", icon: "close", run: () => remove(d, false) },
      d.done > 0 && { label: "Supprimer avec le fichier", icon: "trash", danger: true, run: () => remove(d, true) },
    ]);
  }

  function statusText(d) {
    if (d.st === "done") return `Terminé ${fmtAgo(d.completion_on)}`;
    if (d.st === "paused") return "En pause";
    if (d.st === "queued") return "En attente de son tour…";
    if (d.st === "error") return d.error || "Erreur";
    if (d.retrying) return d.error || "Connexion perdue — nouvel essai…";
    if (!d.size || d.size < 0) return d.speed ? fmtSpeed(d.speed) : "Connexion…";
    return "";
  }
</script>

<section class="dls">
  <form class="add" onsubmit={(e) => (e.preventDefault(), add())}>
    <label class="field">
      <Icon name="link" size={15} />
      <input bind:value={links} placeholder="Colle un ou plusieurs liens directs (https://…/fichier.zip)" spellcheck="false" />
    </label>
    <button class="btn primary" disabled={!links.trim()}><Icon name="down" size={14} />Télécharger</button>
  </form>

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
  </div>

  {#if !downloads.length}
    <div class="empty">
      <Icon name="down" size={34} />
      <p><b>Aucun téléchargement direct</b><br />Colle un lien au-dessus (ou dans le bouton + → Lien). Le téléchargement reprend tout seul après une coupure de connexion ou un redémarrage, avec plusieurs connexions en parallèle.</p>
    </div>
  {:else if !shown.length}
    <p class="hint">Rien ici avec ce filtre.</p>
  {:else}
    <ul>
      {#each shown as d (d.id)}
        {@const art = arts[d.art_key]}
        {@const pct = Math.floor(d.progress * 1000) / 10}
        <li class="st-{d.st}" class:retry={d.retrying} in:fly={{ y: 6, duration: 140 }} oncontextmenu={(e) => menu(e, d)} ondblclick={() => openItem(d)}>
          <span class="thumb k-{d.kind}">
            {#if art?.url}<img src={art.url} alt="" />{:else}<Icon name={d.kind} size={18} />{/if}
          </span>
          <div class="main">
            <div class="top">
              <span class="title" title={d.name}>{d.title || d.name}</span>
              <span class="kb">{KIND_LABEL[d.kind] ?? "Autre"}</span>
            </div>
            <span class="fname" title={d.url}>{d.name}</span>
            {#if d.st !== "done"}
              <div class="pbar"><span style="width:{pct}%"></span></div>
            {/if}
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
          </div>
          <div class="acts">
            {#if d.st === "running" || d.st === "queued"}
              <button class="ib" title="Pause" onclick={() => act([d], "pause")}><Icon name="pause" size={15} /></button>
            {:else if d.st === "paused"}
              <button class="ib hot" title="Reprendre" onclick={() => act([d], "resume")}><Icon name="play" size={15} /></button>
            {:else if d.st === "error"}
              <button class="ib" title="Réessayer" onclick={() => act([d], "resume")}><Icon name="refresh" size={15} /></button>
            {:else}
              <button class="ib" title="Ouvrir" onclick={() => openItem(d)}><Icon name="open" size={15} /></button>
            {/if}
            <button class="ib" title="Ouvrir le dossier" onclick={() => reveal(d.path, d.dir).catch((e) => flash(String(e)))}><Icon name="folder" size={15} /></button>
            <button class="ib" title="Plus d'actions" onclick={(e) => menu(e, d)}><Icon name="more" size={15} /></button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</section>

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
  li { display: flex; align-items: center; gap: 12px; padding: 10px 10px 10px 10px; border-radius: 12px; background: var(--surface); border: 1px solid var(--line); }
  li:hover { border-color: var(--line-strong); }
  li.st-error { border-color: rgb(225 29 72 / 0.35); }
  .thumb { flex: none; width: 42px; height: 60px; border-radius: 7px; overflow: hidden; display: grid; place-items: center; background: var(--chip); color: var(--muted); }
  .thumb img { width: 100%; height: 100%; object-fit: cover; }
  .main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .top { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .title { font-size: 14px; font-weight: 650; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .kb { flex: none; font-size: 10.5px; font-weight: 700; padding: 1px 6px; border-radius: 5px; background: var(--chip); color: var(--text-2); }
  .fname { font-size: 11.5px; color: var(--muted); font-family: ui-monospace, "Cascadia Code", monospace; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
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
  .acts { display: flex; gap: 2px; flex: none; }
  .ib { display: grid; place-items: center; width: 32px; height: 32px; border-radius: 8px; border: 1px solid transparent; background: transparent; color: var(--text-2); cursor: pointer; }
  .ib:hover { background: var(--chip); color: var(--text); }
  .empty { display: flex; flex-direction: column; align-items: center; gap: 12px; padding: 60px 20px; color: var(--muted); text-align: center; }
  .empty p { margin: 0; max-width: 460px; line-height: 1.55; font-size: 13px; }
  .empty b { color: var(--text); font-size: 15px; }
  .hint { color: var(--muted); font-size: 13px; padding: 20px 6px; margin: 0; }
</style>
