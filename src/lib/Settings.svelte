<script>
  import { fade, fly } from "svelte/transition";
  import { listen } from "@tauri-apps/api/event";
  import { open as pickFolder } from "@tauri-apps/plugin-dialog";
  import Icon from "./Icon.svelte";
  import { getDisks, getLimits, getSettings, saveSettings, scanTorrents, setLimits, testNotification, openDefaultApps, getSeeding, setSeeding, setCustomIcon, getCustomIcon, organizeShows, trashFiles, reveal } from "./api.js";
  import { KINDS, THEMES, fmtDate, fmtSize, fmtSpeed } from "./model.js";
  import { confirmAsk, flash, ui } from "./ui.svelte.js";
  import { upd, checkUpdates, installUpdate } from "./updates.svelte.js";
  import { chooseExe, emus, installEmu, loadEmus, removeEmu } from "./emu.svelte.js";
  import { openUrl } from "./api.js";

  let { prefs, torrents, tab = $bindable("general"), onclose } = $props();

  const TABS = [
    { id: "general", label: "Général", icon: "settings" },
    { id: "folders", label: "Dossiers", icon: "folder" },
    { id: "storage", label: "Stockage", icon: "drive" },
    { id: "torrents", label: "Fichiers .torrent", icon: "torrent" },
    { id: "emulators", label: "Émulateurs", icon: "game" },
  ];
  const MEDIA_KINDS = KINDS.filter((k) => k.id !== "all" && k.id !== "other");
  const MB = 1024 * 1024;

  // ---------- réglages ----------
  let settings = $state(null);
  let saving = $state(false);
  let dl = $state(0); // Mo/s, 0 = illimité
  let up = $state(0);

  $effect(() => {
    getSettings().then((s) => (settings = s)).catch((e) => flash(String(e)));
    getLimits()
      .then(([d, u]) => ((dl = +(d / MB).toFixed(2)), (up = +(u / MB).toFixed(2))))
      .catch(() => {});
  });

  async function save() {
    saving = true;
    try {
      await saveSettings($state.snapshot(settings));
      flash("Réglages enregistrés");
    } catch (e) {
      flash(String(e));
    } finally {
      saving = false;
    }
  }

  // Vérifie les mises à jour à l'ouverture des Réglages (si la dernière vérification date de plus de 10 min)
  $effect(() => {
    if (!upd.checking && Date.now() - (upd.lastCheck ?? 0) > 10 * 60 * 1000) checkUpdates();
  });

  // Icône personnalisée (sur ce PC)
  let customIcon = $state(null);
  $effect(() => {
    getCustomIcon().then((b) => (customIcon = b ? `data:image/png;base64,${b}` : null)).catch(() => {});
  });
  function iconChanged(b64) {
    customIcon = b64 ? `data:image/png;base64,${b64}` : null;
    window.dispatchEvent(new CustomEvent("oneforall-icon", { detail: customIcon }));
  }
  async function pickIcon() {
    const file = await pickFolder({ multiple: false, filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp"] }] });
    if (typeof file !== "string") return;
    try {
      iconChanged(await setCustomIcon(file));
      flash("Icône appliquée ✓ (les raccourcis se mettent à jour dans quelques secondes)");
    } catch (e) {
      flash(String(e));
    }
  }
  async function resetIcon() {
    try {
      iconChanged(await setCustomIcon(null));
      flash("Icône d'origine remise");
    } catch (e) {
      flash(String(e));
    }
  }

  // Un dossier par série
  let organizing = $state(false);
  async function organizeAll() {
    const ok = await confirmAsk(
      "Ranger chaque série dans son dossier ?",
      "Les épisodes d'une même série sont déplacés dans un dossier à son nom (ex. D:\\Series\\JoJo's Bizarre Adventure). Les torrents continuent de fonctionner.",
      "Ranger",
    );
    if (!ok) return;
    organizing = true;
    try {
      const n = await organizeShows(null);
      flash(n ? `${n} épisode${n > 1 ? "s" : ""} rangé${n > 1 ? "s" : ""} ✓` : "Tout est déjà rangé ✓");
    } catch (e) {
      flash(String(e));
    } finally {
      organizing = false;
    }
  }

  // Partage (seed)
  let seeding = $state(true);
  $effect(() => {
    getSeeding().then((s) => (seeding = s)).catch(() => {});
  });
  async function toggleSeed(enabled) {
    seeding = enabled;
    try {
      await setSeeding(enabled);
      flash(enabled ? "Partage réactivé" : "Partage désactivé");
    } catch (e) {
      seeding = !enabled;
      flash(String(e));
    }
  }

  async function applyLimits() {
    try {
      await setLimits(Math.round((+dl || 0) * MB), Math.round((+up || 0) * MB));
      flash(`Limites : ↓ ${+dl ? fmtSpeed(dl * MB) : "illimité"} · ↑ ${+up ? fmtSpeed(up * MB) : "illimité"}`);
    } catch (e) {
      flash(String(e));
    }
  }

  async function browse(kind) {
    const dir = await pickFolder({ directory: true, defaultPath: settings.folders[kind] || undefined });
    if (typeof dir === "string") settings.folders[kind] = dir;
  }

  // ---------- stockage ----------
  let disks = $state([]);
  $effect(() => {
    if (tab === "storage") getDisks().then((d) => (disks = d));
    if (tab === "emulators") loadEmus();
  });
  const drive = (p) => (p.match(/^[a-z]:/i)?.[0] ?? "").toUpperCase();
  const usage = $derived.by(() => {
    const byKind = {};
    const byDrive = {};
    let total = 0;
    for (const t of torrents) {
      const b = t.completed;
      total += b;
      byKind[t.kind] = (byKind[t.kind] ?? 0) + b;
      const d = drive(t.save_path);
      byDrive[d] = (byDrive[d] ?? 0) + b;
    }
    const biggest = [...torrents].sort((a, b) => b.completed - a.completed).slice(0, 6);
    return { byKind, byDrive, total, biggest };
  });
  const kindMax = $derived(Math.max(1, ...Object.values(usage.byKind)));

  // ---------- fichiers .torrent ----------
  let scan = $state({ running: false, dirs: 0, items: null, show: "unused" });
  let checked = $state(new Set());

  async function runScan() {
    scan.running = true;
    scan.dirs = 0;
    checked = new Set();
    const un = await listen("scan-progress", (e) => (scan.dirs = e.payload));
    try {
      scan.items = await scanTorrents();
      checked = new Set(scan.items.filter((i) => !i.in_use).map((i) => i.path));
    } catch (e) {
      flash(String(e));
    } finally {
      un();
      scan.running = false;
    }
  }

  const unused = $derived(scan.items?.filter((i) => !i.in_use) ?? []);
  const used = $derived(scan.items?.filter((i) => i.in_use) ?? []);
  const list = $derived(scan.show === "unused" ? unused : used);
  const allChecked = $derived(list.length > 0 && list.every((i) => checked.has(i.path)));

  function toggle(p) {
    const s = new Set(checked);
    s.has(p) ? s.delete(p) : s.add(p);
    checked = s;
  }
  function toggleAll() {
    const s = new Set(checked);
    for (const i of list) allChecked ? s.delete(i.path) : s.add(i.path);
    checked = s;
  }

  async function trashSelected() {
    const paths = list.filter((i) => checked.has(i.path)).map((i) => i.path);
    if (!paths.length) return;
    const inUse = paths.filter((p) => used.some((u) => u.path === p)).length;
    const ok = await confirmAsk(
      `Envoyer ${paths.length} fichier${paths.length > 1 ? "s" : ""} .torrent à la Corbeille ?`,
      inUse ? `Attention : ${inUse} d'entre eux sont encore utilisés par qBittorrent.` : "Tu pourras les récupérer depuis la Corbeille.",
      "Mettre à la Corbeille",
      true,
    );
    if (!ok) return;
    try {
      const n = await trashFiles(paths);
      flash(`${n} fichier${n > 1 ? "s" : ""} envoyé${n > 1 ? "s" : ""} à la Corbeille`);
      const gone = new Set(paths);
      scan.items = scan.items.filter((i) => !gone.has(i.path));
      checked = new Set([...checked].filter((p) => !gone.has(p)));
    } catch (e) {
      flash(String(e));
    }
  }
</script>

<div class="backdrop" transition:fade={{ duration: 140 }} onclick={onclose} role="presentation"></div>
<div class="modal" transition:fly={{ y: 16, duration: 200 }} role="dialog" aria-label="Réglages">
  <nav>
    <h2>Réglages</h2>
    {#each TABS as t}
      <button class:on={tab === t.id} onclick={() => (tab = t.id)}><Icon name={t.icon} size={16} />{t.label}</button>
    {/each}

    <!-- Version et mises à jour : une ligne, vérification automatique à l'ouverture -->
    {#if upd.available}
      <!-- Mise à jour dispo : une carte cliquable, la progression s'affiche dedans -->
      <button class="updcard" disabled={upd.progress != null} onclick={installUpdate} title="Installer la v{upd.available.version} (l'app redémarre)">
        <span class="uc-ico"><Icon name="down" size={15} /></span>
        <span class="uc-txt">
          <b>{upd.progress != null ? "Installation…" : "Mise à jour dispo"}</b>
          <small>v{upd.current} → v{upd.available.version}</small>
        </span>
        {#if upd.progress != null}<span class="uc-pct">{Math.round((upd.progress ?? 0) * 100)} %</span>{/if}
        {#if upd.progress != null}<span class="uc-bar" style="width: {Math.round((upd.progress ?? 0) * 100)}%"></span>{/if}
      </button>
    {:else}
      <div class="about">
        <span class="ver" title={upd.error ?? ""}>
          v{upd.current}
          {#if upd.checking}<i class="st">…</i>{:else if upd.error}<i class="st bad">hors ligne</i>{:else if upd.checkedOnce}<i class="st ok">à jour</i>{/if}
        </span>
        <button class="upd ico" class:spinning={upd.checking} disabled={upd.checking} title="Rechercher une mise à jour" onclick={() => checkUpdates().then((a) => !a && !upd.error && flash("OneForAll est à jour"))}>
          <Icon name="refresh" size={13} />
        </button>
      </div>
    {/if}
  </nav>

  <section>
    <button class="icon-btn close" title="Fermer (Échap)" onclick={onclose}><Icon name="close" size={16} /></button>

    {#if tab === "general"}
      <!-- Apparence -->
      <div class="card">
        <h3><Icon name="sun" size={15} />Apparence</h3>
        <div class="srow">
          <span class="sl">Thème</span>
          <div class="themes">
            {#each THEMES as t}
              <button class="theme" class:on={prefs.theme === t.id} onclick={() => (prefs.theme = t.id)} title={t.label}>
                <span class="tsw" style="background:{t.sw[0]}"><i style="background:{t.sw[1]}"></i><i style="background:{t.sw[1]};opacity:.45"></i></span>
                <span class="tname"><Icon name={t.icon} size={12} />{t.label}</span>
              </button>
            {/each}
          </div>
        </div>
        <div class="srow">
          <span class="sl">Icône de l'app <i class="info" title="N'importe quelle image : bords unis retirés, recadrée en carré. Reste uniquement sur ce PC (fenêtre, barre des tâches, raccourcis).">i</i></span>
          <div class="ctl">
            {#if customIcon}<img class="icon-prev" src={customIcon} alt="" />{/if}
            <button class="btn sm" onclick={pickIcon}><Icon name="image" size={14} />Choisir…</button>
            {#if customIcon}<button class="btn sm" title="Icône d'origine" onclick={resetIcon}><Icon name="refresh" size={13} /></button>{/if}
          </div>
        </div>
      </div>

      <!-- Vitesse -->
      <div class="card">
        <h3><Icon name="gauge" size={15} />Vitesse <i class="info" title="0 = illimité. S'applique tout de suite à qBittorrent.">i</i></h3>
        <div class="limits">
          <label><Icon name="down" size={14} /><div class="unit"><input type="number" min="0" step="0.5" bind:value={dl} onchange={applyLimits} /><em>Mo/s</em></div></label>
          <label><Icon name="up" size={14} /><div class="unit"><input type="number" min="0" step="0.5" bind:value={up} onchange={applyLimits} /><em>Mo/s</em></div></label>
        </div>
        <div class="presets">
          {#each [0, 1, 5, 10, 25, 50] as v}
            <button class="chip-sm" class:on={+dl === v} onclick={() => ((dl = v), applyLimits())}>{v ? `${v} Mo/s` : "∞"}</button>
          {/each}
        </div>
      </div>

      {#if settings}
        <!-- Téléchargements -->
        <div class="card">
          <h3><Icon name="down" size={15} />Téléchargements</h3>
          <label class="srow">
            <span class="sl"><Icon name="up" size={14} />Désactiver le partage (seed) <i class="info" title="Les torrents s'arrêtent dès la fin du téléchargement. Attention au ratio sur les trackers privés.">i</i></span>
            <input type="checkbox" class="switch" checked={!seeding} onchange={(e) => toggleSeed(!e.currentTarget.checked)} />
          </label>
          <label class="srow">
            <span class="sl"><Icon name="anime" size={14} />Séparer animés et séries <i class="info" title="Désactivé : les animés sont rangés et affichés comme des séries.">i</i></span>
            <input type="checkbox" class="switch" bind:checked={settings.split_anime} onchange={save} />
          </label>
        </div>

        <!-- Intégrations -->
        <div class="card">
          <h3><Icon name="link" size={15} />Intégrations</h3>
          <div class="srow">
            <span class="sl">C411 <i class="info" title="Clé API (droit « Torznab/RSS (lecture) ») pour chercher sur C411 depuis le bouton +. À créer sur c411.org → Intégrations API. Reste sur ce PC.">i</i></span>
            <div class="ctl grow">
              <input class="text" type="password" placeholder="Clé API C411" bind:value={settings.c411_key} spellcheck="false" autocomplete="off" />
              <button class="btn sm" onclick={save}><Icon name="check" size={13} /></button>
            </div>
          </div>
          <div class="srow">
            <span class="sl">TMDB <i class="info" title="Facultatif : une clé est déjà intégrée. La tienne sera utilisée à la place.">i</i></span>
            <div class="ctl grow">
              <input class="text" type="password" placeholder="Clé personnelle (facultatif)" bind:value={settings.tmdb_key} spellcheck="false" autocomplete="off" />
              <button class="btn sm" onclick={save}><Icon name="check" size={13} /></button>
            </div>
          </div>
          <div class="srow">
            <span class="sl">Liens magnet <i class="info" title="Les .torrent s'ouvrent déjà dans OneForAll. Pour les liens magnet, Windows demande une fois : choisis OneForAll.">i</i></span>
            <button class="btn sm" onclick={() => openDefaultApps().catch((e) => flash(String(e)))}><Icon name="open" size={13} />Régler…</button>
          </div>
        </div>
      {/if}
    {:else if tab === "folders"}
      <h3>Dossier par défaut de chaque type</h3>
      <p class="hint">
        Utilisé par le rangement automatique (une catégorie qBittorrent par type). Les torrents déjà présents ne sont pas déplacés.
      </p>
      {#if settings}
        <div class="folders">
          {#each MEDIA_KINDS as k}
            <div class="folder">
              <span class="fk"><Icon name={k.id} size={16} />{k.label}</span>
              <input class="text" bind:value={settings.folders[k.id]} placeholder="Non défini" spellcheck="false" />
              <button class="btn" onclick={() => browse(k.id)}>Parcourir…</button>
            </div>
          {/each}
        </div>
        <h3 class="dlh">Onglet DL</h3>
        <p class="hint">Tous les téléchargements de l'onglet DL (fichiers, vidéos, musiques) arrivent ici, quel que soit leur type. Les extraits du studio vont dans leur propre dossier.</p>
        <div class="folders">
          <div class="folder">
            <span class="fk"><Icon name="download" size={16} />DL</span>
            <input class="text" bind:value={settings.folders.dl} placeholder="Téléchargements (par défaut)" spellcheck="false" />
            <button class="btn" onclick={() => browse("dl")}>Parcourir…</button>
          </div>
          <div class="folder">
            <span class="fk"><Icon name="scissors" size={16} />Extraits</span>
            <input class="text" bind:value={settings.folders.clips} placeholder="Vidéos\OneForAll Extraits (par défaut)" spellcheck="false" />
            <button class="btn" onclick={() => browse("clips")}>Parcourir…</button>
          </div>
        </div>
        <div class="actions">
          <button class="btn primary" disabled={saving} onclick={save}>{saving ? "Enregistrement…" : "Enregistrer"}</button>
        </div>
      {/if}
    {:else if tab === "storage"}
      <div class="card organize">
        <div class="srow">
          <span class="sl"><Icon name="folder" size={15} />Un dossier par série <i class="info" title="Les épisodes d'une même série (torrents séparés) sont regroupés dans un dossier à son nom, ex. D:\Series\JoJo's Bizarre Adventure. Les nouveaux épisodes y vont automatiquement.">i</i></span>
          <button class="btn sm primary" disabled={organizing} onclick={organizeAll}>
            <Icon name="folder" size={13} />{organizing ? "Rangement…" : "Ranger les séries"}
          </button>
        </div>
      </div>

      <h3>Disques</h3>
      <div class="disks">
        {#each disks as d}
          {@const used = d.total - d.free}
          {@const mine = usage.byDrive[d.mount.replace(/\\$/, "").toUpperCase()] ?? 0}
          <div class="disk">
            <div class="dh">
              <b><Icon name="drive" size={15} />{d.mount.replace(/\\$/, "")} {d.name ? `· ${d.name}` : ""}</b>
              <span>{fmtSize(d.free)} libres sur {fmtSize(d.total)}</span>
              <button class="btn sm analyze" onclick={() => (ui.diskMap = { mount: d.mount, used: d.total - d.free })}><Icon name="pie" size={14} />Analyser</button>
            </div>
            <div class="dbar">
              <span class="other" style="width:{((used - mine) / d.total) * 100}%"></span>
              <span class="mine" style="width:{(mine / d.total) * 100}%"></span>
            </div>
            <div class="legend">
              <span><i class="mine"></i>Torrents : {fmtSize(mine)}</span>
              <span><i class="other"></i>Autres fichiers : {fmtSize(Math.max(0, used - mine))}</span>
            </div>
          </div>
        {/each}
      </div>

      <h3>Torrents par type · {fmtSize(usage.total)} au total</h3>
      <div class="kinds">
        {#each KINDS.filter((k) => usage.byKind[k.id]) as k}
          <div class="krow">
            <span class="fk"><Icon name={k.id} size={15} />{k.label}</span>
            <div class="kbar"><span style="width:{(usage.byKind[k.id] / kindMax) * 100}%"></span></div>
            <b>{fmtSize(usage.byKind[k.id])}</b>
          </div>
        {/each}
      </div>

      <h3>Les plus gros</h3>
      <ul class="big-list">
        {#each usage.biggest as t}
          <li><span title={t.name}>{t.title}</span><b>{fmtSize(t.completed)}</b></li>
        {/each}
      </ul>
    {:else if tab === "emulators"}
      <h3>Émulateurs</h3>
      <p class="hint">
        Double-clique sur un jeu (.nds, .gba, .3ds, .nsp, .iso…) ou clique sur « Jouer » dans un jeu console : OneForAll le lance dans le bon
        émulateur, et l'installe d'abord s'il le faut (version officielle, dans ton dossier OneForAll).
      </p>
      <div class="emus">
        {#each emus.list as e (e.id)}
          {@const pr = emus.progress[e.id]}
          <div class="emu">
            <div class="et">
              <b>{e.name}</b>
              <span>{e.consoles}</span>
              {#if pr}<span class="ep">{pr.msg} {pr.pct ? `${pr.pct} %` : ""}</span>
              {:else if e.installed}<span class="eok" title={e.installed.exe}><Icon name="check" size={12} />Installé{e.installed.version === "perso" ? " (ton exe)" : e.installed.version && e.installed.version !== "latest" ? ` · ${e.installed.version}` : ""}</span>{/if}
            </div>
            <div class="ea">
              {#if e.installed}
                <button class="btn sm" title="Ouvrir son dossier" onclick={() => reveal(e.installed.exe, e.installed.exe)}><Icon name="folder" size={13} /></button>
                {#if !e.manual && e.installed.version !== "perso"}<button class="btn sm" disabled={!!pr} title="Retélécharger la dernière version" onclick={() => installEmu(e.id)}><Icon name="refresh" size={13} /></button>{/if}
                <button class="btn sm" title="Retirer" onclick={() => removeEmu(e.id)}><Icon name="trash" size={13} /></button>
              {:else if e.manual}
                <button class="btn sm" onclick={() => openUrl(e.page)}><Icon name="open" size={13} />Site</button>
                <button class="btn sm" onclick={() => chooseExe(e.id)}>Choisir l'exe…</button>
              {:else}
                <button class="btn sm" disabled={!!pr} onclick={() => chooseExe(e.id)} title="Tu l'as déjà ? Indique son .exe">J'ai déjà</button>
                <button class="btn sm primary" disabled={!!pr} onclick={() => installEmu(e.id)}><Icon name="download" size={13} />Installer</button>
              {/if}
            </div>
          </div>
        {/each}
      </div>
    {:else if tab === "torrents"}
      <h3>Fichiers .torrent sur tes disques</h3>
      <p class="hint">
        Cherche tous les fichiers .torrent (hors dossiers système et dossiers internes de qBittorrent) et indique ceux qui ne
        correspondent plus à aucun torrent actif. La suppression les envoie à la Corbeille.
      </p>
      <div class="row">
        <button class="btn primary" disabled={scan.running} onclick={runScan}>
          <Icon name="search" size={15} />{scan.running ? "Analyse en cours…" : scan.items ? "Relancer l'analyse" : "Analyser mes disques"}
        </button>
        {#if scan.running}<span class="hint inline"><span class="spinner"></span>{scan.dirs.toLocaleString("fr-FR")} dossiers parcourus</span>{/if}
      </div>

      {#if scan.items}
        <div class="scan-head">
          <div class="seg">
            <button class:on={scan.show === "unused"} onclick={() => (scan.show = "unused")}>Inutilisés <span class="n">{unused.length}</span></button>
            <button class:on={scan.show === "used"} onclick={() => (scan.show = "used")}>Utilisés <span class="n">{used.length}</span></button>
          </div>
          <span class="spacer"></span>
          {#if list.length}
            <button class="link" onclick={toggleAll}>{allChecked ? "Tout décocher" : "Tout cocher"}</button>
            <button class="btn danger" disabled={!list.some((i) => checked.has(i.path))} onclick={trashSelected}>
              <Icon name="trash" size={14} />Corbeille ({list.filter((i) => checked.has(i.path)).length})
            </button>
          {/if}
        </div>
        {#if !list.length}
          <p class="hint">{scan.show === "unused" ? "Aucun .torrent inutilisé. Tout est propre ✨" : "Aucun."}</p>
        {:else}
          <ul class="tlist">
            {#each list as i (i.path)}
              <li>
                <label class="check">
                  <input type="checkbox" checked={checked.has(i.path)} onchange={() => toggle(i.path)} />
                  <span class="box"><Icon name="check" size={12} /></span>
                </label>
                <div class="tbody">
                  <span class="tname">{i.name}</span>
                  <span class="tpath" title={i.path}>{i.path}</span>
                </div>
                <span class="tmeta">{fmtDate(i.modified)}</span>
                <button class="mini" title="Afficher dans l'explorateur" onclick={() => reveal(i.path, "")}><Icon name="folder" size={14} /></button>
              </li>
            {/each}
          </ul>
        {/if}
      {/if}
    {/if}
  </section>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 60; background: var(--scrim); backdrop-filter: blur(3px); }
  .modal {
    position: fixed;
    z-index: 61;
    inset: 5vh 50% auto auto;
    translate: 50% 0;
    width: min(980px, 94vw);
    height: min(720px, 90vh);
    display: flex;
    border-radius: 18px;
    overflow: hidden;
    background: var(--bg-2);
    border: 1px solid var(--line-strong);
    box-shadow: var(--shadow-lg);
  }
  nav { width: 220px; flex: none; padding: 22px 12px; background: var(--bg); border-right: 1px solid var(--line); display: flex; flex-direction: column; gap: 2px; }
  nav h2 { margin: 0 10px 16px; font-size: 18px; }
  nav button { display: flex; align-items: center; gap: 10px; padding: 9px 12px; border: 0; border-radius: 9px; background: none; color: var(--text-2); font: inherit; font-size: 13.5px; font-weight: 550; cursor: pointer; text-align: left; }
  nav button:hover { background: var(--chip); color: var(--text); }
  nav button.on { background: var(--accent-soft); color: var(--accent); }
  .icon-row { gap: 10px; }
  .card.organize { margin-bottom: 18px; }
  .icon-prev { width: 44px; height: 44px; border-radius: 10px; box-shadow: var(--shadow-sm); }
  /* version + mise à jour : une seule ligne compacte */
  .about { margin-top: auto; display: flex; align-items: center; gap: 6px; padding: 10px 6px 2px 10px; border-top: 1px solid var(--line); }
  .ver { flex: 1; display: flex; align-items: center; gap: 6px; font-size: 12px; font-weight: 700; color: var(--text-2); font-variant-numeric: tabular-nums; }
  .st { font-style: normal; font-size: 10.5px; font-weight: 650; padding: 1px 6px; border-radius: 999px; background: var(--chip); color: var(--muted); }
  .st.ok { background: rgb(16 185 129 / 0.12); color: var(--ok); }
  .st.bad { background: var(--err-soft); color: var(--err); }
  nav .upd { display: inline-flex; align-items: center; gap: 4px; height: 26px; padding: 0 8px; border-radius: 7px; border: 1px solid var(--line-strong); background: var(--surface); font-size: 11.5px; font-weight: 700; color: var(--text-2); }
  nav .upd.ico { width: 26px; padding: 0; justify-content: center; }
  nav .upd:hover:not(:disabled) { border-color: var(--accent); color: var(--accent); }
  nav .updcard { position: relative; overflow: hidden; margin-top: auto; display: flex; align-items: center; gap: 9px; width: 100%; padding: 9px 10px; border-radius: 10px; border: 1px solid color-mix(in srgb, var(--accent) 30%, transparent); background: var(--accent-soft); color: var(--text); text-align: left; transition: border-color 0.15s, transform 0.15s; }
  nav .updcard:hover { background: var(--accent-soft); color: var(--text); }
  nav .updcard:hover:not(:disabled) { border-color: var(--accent); transform: translateY(-1px); }
  .uc-ico { flex: none; display: grid; place-items: center; width: 28px; height: 28px; border-radius: 8px; background: var(--accent); color: white; }
  .uc-txt { flex: 1; min-width: 0; display: flex; flex-direction: column; line-height: 1.25; }
  .uc-txt b { font-size: 12px; font-weight: 700; color: var(--accent); white-space: nowrap; }
  .uc-txt small { font-size: 11px; color: var(--muted); font-variant-numeric: tabular-nums; white-space: nowrap; }
  .uc-pct { font-size: 11px; font-weight: 700; color: var(--accent); font-variant-numeric: tabular-nums; }
  .uc-bar { position: absolute; left: 0; bottom: 0; height: 3px; background: var(--accent); transition: width 0.2s; }
  .upd.spinning :global(svg) { animation: spin 0.8s linear infinite; }

  /* sections en cartes */
  .card { margin-bottom: 14px; padding: 6px 14px 8px; border-radius: 14px; background: var(--surface); border: 1px solid var(--line); }
  .card h3 { display: flex; align-items: center; gap: 8px; margin: 8px 0 6px; font-size: 12px; color: var(--text-2); }
  .srow { display: flex; align-items: center; justify-content: space-between; gap: 14px; min-height: 42px; padding: 4px 0; border-top: 1px solid var(--line); cursor: default; }
  label.srow { cursor: pointer; }
  .card h3 + .srow { border-top: 0; }
  .sl { display: inline-flex; align-items: center; gap: 8px; font-size: 13px; font-weight: 550; color: var(--text); }
  .ctl { display: flex; align-items: center; gap: 6px; }
  .ctl.grow { flex: 1; max-width: 380px; }
  .info { display: inline-grid; place-items: center; width: 15px; height: 15px; border-radius: 50%; border: 1.5px solid var(--line-strong); font-style: normal; font-size: 9.5px; font-weight: 800; color: var(--muted); cursor: help; }
  .info:hover { border-color: var(--accent); color: var(--accent); }
  :global(.btn.sm) { height: 30px; padding: 0 10px; font-size: 12.5px; }
  .card .limits { grid-template-columns: 1fr 1fr; margin: 4px 0 2px; }
  .card .limits label { flex-direction: row; align-items: center; gap: 10px; color: var(--muted); }
  .card .limits .unit { flex: 1; }
  .card .presets { margin: 8px 0 6px; }
  section { flex: 1; overflow-y: auto; padding: 26px 32px 32px; position: relative; }
  .close { position: absolute; top: 16px; right: 16px; }
  h3 { margin: 22px 0 10px; font-size: 13px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
  h3:first-of-type { margin-top: 4px; }
  .hint { margin: -4px 0 12px; color: var(--muted); font-size: 12.5px; line-height: 1.5; }
  .hint.inline { margin: 0; display: inline-flex; align-items: center; gap: 8px; }
  .row { display: flex; gap: 10px; align-items: center; }
  .spacer { flex: 1; }
  .text { flex: 1; height: 36px; padding: 0 12px; border-radius: 9px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: inherit; font-size: 13px; outline: none; min-width: 0; }
  .text:focus, .unit input:focus { border-color: var(--accent); box-shadow: 0 0 0 3px var(--ring); }

  .emus { display: flex; flex-direction: column; gap: 6px; margin-top: 12px; }
  .emu { display: flex; align-items: center; gap: 12px; padding: 10px 12px; border-radius: 11px; background: var(--surface); border: 1px solid var(--line); }
  .et { flex: 1; min-width: 0; display: flex; align-items: baseline; flex-wrap: wrap; gap: 4px 10px; }
  .et b { font-size: 13.5px; }
  .et span { font-size: 12px; color: var(--muted); }
  .et .eok { display: inline-flex; align-items: center; gap: 4px; color: var(--ok); font-weight: 650; }
  .et .ep { color: var(--accent); font-weight: 650; }
  .ea { display: flex; gap: 6px; flex: none; }
  .themes { display: flex; flex-wrap: wrap; gap: 8px; justify-content: flex-end; }
  .theme { display: flex; flex-direction: column; gap: 6px; padding: 5px; border-radius: 11px; border: 1.5px solid var(--line-strong); background: var(--surface); color: var(--text-2); font: inherit; cursor: pointer; transition: border-color 0.12s, transform 0.12s; }
  .theme:hover { border-color: var(--accent); transform: translateY(-1px); }
  .theme.on { border-color: var(--accent); box-shadow: 0 0 0 3px var(--ring); color: var(--text); }
  .tsw { position: relative; display: flex; align-items: flex-end; gap: 4px; width: 74px; height: 42px; padding: 6px; border-radius: 7px; box-shadow: inset 0 0 0 1px rgb(0 0 0 / 0.08); }
  .tsw i { display: block; height: 8px; border-radius: 4px; flex: 1; }
  .tsw i:first-child { flex: 2; }
  .tname { display: inline-flex; align-items: center; justify-content: center; gap: 5px; font-size: 12px; font-weight: 650; }
  .seg { display: inline-flex; padding: 3px; border-radius: 10px; background: var(--chip); }
  .seg button { display: inline-flex; align-items: center; gap: 6px; height: 30px; padding: 0 14px; border: 0; border-radius: 7px; background: none; color: var(--muted); font: inherit; font-size: 13px; font-weight: 550; cursor: pointer; }
  .seg button.on { background: var(--surface); color: var(--text); box-shadow: var(--shadow-sm); }
  .seg.big button { height: 34px; padding: 0 18px; }
  .n { font-size: 11px; font-weight: 700; opacity: 0.6; }

  .limits { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .limits label { display: flex; flex-direction: column; gap: 6px; }
  .limits span { display: inline-flex; align-items: center; gap: 6px; font-size: 13px; font-weight: 600; }
  .unit { display: flex; align-items: center; position: relative; }
  .unit input { width: 100%; height: 38px; padding: 0 56px 0 12px; border-radius: 9px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: inherit; font-size: 14px; outline: none; }
  .unit em { position: absolute; right: 12px; font-style: normal; color: var(--muted); font-size: 12px; }
  .presets { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 10px; }
  .chip-sm { height: 28px; padding: 0 11px; border-radius: 999px; border: 1px solid var(--line-strong); background: none; color: var(--text-2); font: inherit; font-size: 12px; cursor: pointer; }
  .chip-sm.on { background: var(--accent); border-color: var(--accent); color: white; }

  .switch-row { display: flex; align-items: center; gap: 20px; padding: 12px 14px; border-radius: 12px; background: var(--surface); border: 1px solid var(--line); margin-bottom: 8px; cursor: pointer; }
  .switch-row div { flex: 1; display: flex; flex-direction: column; gap: 3px; }
  .switch-row b { font-size: 13.5px; }
  .switch-row span { font-size: 12.5px; color: var(--muted); line-height: 1.45; }
  .switch { appearance: none; width: 40px; height: 23px; border-radius: 999px; background: var(--line-strong); position: relative; cursor: pointer; transition: background 0.15s; flex: none; margin: 0; }
  .switch::after { content: ""; position: absolute; top: 3px; left: 3px; width: 17px; height: 17px; border-radius: 50%; background: white; transition: transform 0.15s; box-shadow: 0 1px 3px rgb(0 0 0 / 0.25); }
  .switch:checked { background: var(--accent); }
  .switch:checked::after { transform: translateX(17px); }

  .folders { display: flex; flex-direction: column; gap: 8px; }
  .dlh { margin-top: 22px; }
  .folder { display: flex; align-items: center; gap: 10px; }
  .fk { display: inline-flex; align-items: center; gap: 8px; width: 120px; flex: none; font-weight: 600; font-size: 13px; }
  .actions { display: flex; justify-content: flex-end; margin-top: 18px; }

  .disks { display: flex; flex-direction: column; gap: 12px; }
  .disk { padding: 14px; border-radius: 12px; background: var(--surface); border: 1px solid var(--line); }
  .dh { display: flex; gap: 12px; align-items: center; margin-bottom: 10px; font-size: 13px; }
  .dh b { display: inline-flex; gap: 8px; align-items: center; }
  .dh span { color: var(--muted); margin-left: auto; }
  .dbar { display: flex; height: 10px; border-radius: 6px; background: var(--chip); overflow: hidden; }
  .dbar .mine, .legend i.mine { background: linear-gradient(90deg, var(--accent), var(--accent-2)); }
  .dbar .other, .legend i.other { background: var(--muted); opacity: 0.45; }
  .legend { display: flex; gap: 16px; margin-top: 8px; font-size: 12px; color: var(--muted); }
  .legend i { display: inline-block; width: 9px; height: 9px; border-radius: 3px; margin-right: 6px; }

  .kinds { display: flex; flex-direction: column; gap: 8px; }
  .krow { display: flex; align-items: center; gap: 12px; font-size: 13px; }
  .kbar { flex: 1; height: 8px; border-radius: 5px; background: var(--chip); overflow: hidden; }
  .kbar span { display: block; height: 100%; border-radius: 5px; background: linear-gradient(90deg, var(--accent), var(--accent-2)); }
  .krow b { width: 80px; text-align: right; font-variant-numeric: tabular-nums; }
  .big-list { list-style: none; margin: 0; padding: 0; }
  .big-list li { display: flex; justify-content: space-between; gap: 12px; padding: 8px 2px; border-bottom: 1px solid var(--line); font-size: 13px; }
  .big-list span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .scan-head { display: flex; align-items: center; gap: 12px; margin: 18px 0 10px; }
  .link { border: 0; background: none; color: var(--accent); font: inherit; font-size: 12.5px; font-weight: 600; cursor: pointer; }
  .tlist { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .tlist li { display: flex; align-items: center; gap: 12px; padding: 8px 10px; border-radius: 9px; }
  .tlist li:hover { background: var(--chip); }
  .tbody { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .tname { font-size: 13px; font-weight: 550; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .tpath { font-size: 11.5px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-family: ui-monospace, "Cascadia Code", monospace; }
  .tmeta { font-size: 12px; color: var(--muted); white-space: nowrap; }
  .mini { display: grid; place-items: center; width: 28px; height: 28px; border-radius: 7px; border: 0; background: transparent; color: var(--muted); cursor: pointer; }
  .mini:hover { background: var(--chip); color: var(--text); }

  .check { position: relative; display: grid; place-items: center; cursor: pointer; flex: none; }
  .check input { position: absolute; opacity: 0; width: 18px; height: 18px; margin: 0; cursor: pointer; }
  .box { width: 18px; height: 18px; border-radius: 5px; border: 1.5px solid var(--line-strong); display: grid; place-items: center; color: transparent; }
  .check input:checked + .box { background: var(--accent); border-color: var(--accent); color: white; }

  .spinner { width: 14px; height: 14px; border-radius: 50%; border: 2px solid var(--line-strong); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>