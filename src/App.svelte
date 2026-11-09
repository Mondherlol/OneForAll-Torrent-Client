<script>
  import { flip } from "svelte/animate";
  import { fade, fly } from "svelte/transition";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import Card from "./lib/Card.svelte";
  import Drawer from "./lib/Drawer.svelte";
  import Settings from "./lib/Settings.svelte";
  import ContextMenu from "./lib/ContextMenu.svelte";
  import Confirm from "./lib/Confirm.svelte";
  import Play from "./lib/Play.svelte";
  import { emuTakePending } from "./lib/api.js";
  import AddTorrent from "./lib/AddTorrent.svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { listen } from "@tauri-apps/api/event";
  import { stageTorrents, qbitStatus, setupQbit, openUrl, getCustomIcon, organizeShows } from "./lib/api.js";
  import PickFiles from "./lib/PickFiles.svelte";
  import Downloads from "./lib/Downloads.svelte";
  import Studio from "./lib/Studio.svelte";
  import { getDownloads, emuDetect, downloadAction, dlExtract, romLibrary, romScan, trashPath } from "./lib/api.js";
  import { play as playGame } from "./lib/emu.svelte.js";
  import logo from "./assets/logo.png";
  import { upd, checkUpdates, installUpdate } from "./lib/updates.svelte.js";

  // Vérifie les mises à jour quelques secondes après le démarrage, puis toutes les 6 h
  $effect(() => {
    const t = setTimeout(() => checkUpdates(), 5000);
    const id = setInterval(() => checkUpdates(), 6 * 3600 * 1000);
    return () => (clearTimeout(t), clearInterval(id));
  });
  let updDismissed = $state(false);

  // Icône personnalisée (sur ce PC) : aussi utilisée comme logo en haut à gauche
  let customLogo = $state(null);
  $effect(() => {
    getCustomIcon().then((b) => (customLogo = b ? `data:image/png;base64,${b}` : null)).catch(() => {});
    const on = (e) => (customLogo = e.detail);
    window.addEventListener("oneforall-icon", on);
    return () => window.removeEventListener("oneforall-icon", on);
  });

  // ---------- qBittorrent pas encore relié ----------
  let qstatus = $state(null);
  let settingUp = $state(false);
  $effect(() => {
    if (error && !torrents.length && !booting) qbitStatus().then((s) => (qstatus = s)).catch(() => {});
  });
  async function autoSetup() {
    settingUp = true;
    try {
      await setupQbit();
      flash("qBittorrent est relié à OneForAll ✓");
      await refresh();
    } catch (e) {
      flash(String(e));
    } finally {
      settingUp = false;
    }
  }

  // Titre de la fenêtre : « OneForAll — Torrent Client v0.2.2 »
  $effect(() => {
    if (upd.current) getCurrentWindow().setTitle(`OneForAll — Torrent Client v${upd.current}`).catch(() => {});
  });
  import Icon from "./lib/Icon.svelte";
  import { open as pickFolder } from "@tauri-apps/plugin-dialog";
  import { getCategories, getDisks, getTorrents, openPath, play, reveal, runSetup, setCategory, setLocation, shareTorrents, torrentAction, uninstallProgram } from "./lib/api.js";
  import { KINDS, PLAYABLE, SORTS, STATUS_FILTERS, THEMES, buildGroups, fmtSize, themeOf, fmtSpeed, isActive, isDone, isPaused, sortGroups } from "./lib/model.js";
  import { confirmAsk, copy, flash, openMenu, openStudio, pickArt, renameGame, ui, withGameName } from "./lib/ui.svelte.js";
  import ArtPicker from "./lib/ArtPicker.svelte";
  import DiskMap from "./lib/DiskMap.svelte";
  import RenameGame from "./lib/RenameGame.svelte";

  const DEFAULTS = { kind: "all", status: "all", sort: "added", dir: "desc", grouped: true, theme: "light" };
  let prefs = $state({ ...DEFAULTS, ...JSON.parse(localStorage.getItem("oneforall.prefs") ?? "{}") });
  $effect(() => localStorage.setItem("oneforall.prefs", JSON.stringify(prefs)));

  let torrents = $state([]);
  // Téléchargements directs (liens http), affichés dans leur propre onglet
  let downloads = $state([]);
  let view = $state("torrents");
  async function refreshDownloads() {
    try {
      downloads = await getDownloads();
    } catch {}
  }
  $effect(() => {
    if (view !== "downloads") return;
    const id = setInterval(() => !document.hidden && refreshDownloads(), 1000);
    return () => clearInterval(id);
  });
  // Jeux téléchargés hors torrent (sites de ROM, liens…) : en cartes sous ceux des torrents, onglet Jeux
  const DL_ST = { running: "downloading", queued: "stalled", paused: "paused", done: "done", error: "error" };
  const dlCards = $derived.by(() => {
    if (prefs.kind !== "game") return [];
    const q = query.trim().toLowerCase();
    return downloads
      .filter((d) => d.kind === "game" && (!q || d.title.toLowerCase().includes(q) || d.name.toLowerCase().includes(q)))
      .map((d) => {
        const status = DL_ST[d.st] ?? "done";
        return {
          key: `dl:${d.id}`,
          dl: d,
          artKey: d.art_key,
          kind: d.kind,
          title: d.title,
          year: d.year,
          seasons: [],
          items: [{ hash: d.id, progress: d.progress, status, format: d.platform ? "rom" : "", platform: d.platform }],
          progress: d.progress,
          status,
          size: d.size,
          added_on: d.added_on,
          completion_on: d.completion_on,
          dlspeed: d.speed,
          eta: d.eta,
        };
      })
      .concat(romCards(q))
      .map(withGameName)
      .filter((g) => matchStatus(g.status, prefs.status))
      .sort((a, b) => b.added_on - a.added_on);
  });

  // ROM déjà sur le PC (bouton « Scanner mes ROM ») : sauf celles d'un torrent ou d'un téléchargement
  let romLib = $state([]);
  let hiddenRoms = $state(new Set(JSON.parse(localStorage.getItem("oneforall.hiddenRoms") ?? "[]")));
  let romScanning = $state(null); // { files, found } pendant le scan
  romLibrary().then((l) => (romLib = l)).catch(() => {});
  function romCards(q) {
    const low = (p) => (p ?? "").toLowerCase();
    const owned = [
      ...torrents.filter((t) => t.content_path).map((t) => low(t.content_path)),
      ...downloads.flatMap((d) => [low(d.path), low(d.rom), low(d.path).replace(/\.(zip|7z)$/, "")]).filter(Boolean),
    ];
    return romLib
      .filter((r) => !hiddenRoms.has(r.path) && (!q || r.title.toLowerCase().includes(q)))
      .filter((r) => !owned.some((o) => low(r.path) === o || low(r.path).startsWith(o + "\\")))
      .map((r) => ({
        key: `rom:${r.path}`,
        rom: r,
        artKey: r.art_key,
        kind: "game",
        title: r.title,
        year: null,
        seasons: [],
        items: [{ hash: r.path, progress: 1, status: "done", format: "rom", platform: r.platform }],
        progress: 1,
        status: "done",
        size: r.size,
        added_on: r.added_on,
        completion_on: r.added_on,
        dlspeed: 0,
        eta: 0,
      }));
  }
  async function scanRoms() {
    romScanning = { files: 0, found: 0 };
    const un = await listen("rom-scan", (e) => romScanning && (romScanning = e.payload));
    try {
      const before = new Set(romLib.map((r) => r.path));
      romLib = await romScan();
      const added = romLib.filter((r) => !before.has(r.path)).length;
      flash(added ? `${added} jeu${added > 1 ? "x" : ""} trouvé${added > 1 ? "s" : ""} sur le PC` : "Aucun nouveau jeu trouvé");
    } catch (e) {
      flash(String(e));
    } finally {
      un();
      romScanning = null;
    }
  }
  function hideRom(r) {
    hiddenRoms = new Set([...hiddenRoms, r.path]);
    localStorage.setItem("oneforall.hiddenRoms", JSON.stringify([...hiddenRoms]));
  }
  const romPlay = (r) => playGame({ path: r.path, name: r.title, emu: r.emu, size: r.size });
  function romCardMenu(e, g) {
    const r = g.rom;
    openMenu(e, [
      { label: "Jouer", icon: "play", run: () => romPlay(r) },
      { label: "Vrai nom du jeu…", icon: "tag", run: () => renameGame(g) },
      { label: "Choisir l'image…", icon: "image", run: () => pickArt(g) },
      { label: "Ouvrir le dossier", icon: "folder", run: () => reveal(r.path, null).catch((err) => flash(String(err))) },
      "-",
      { label: "Masquer ce jeu", icon: "close", run: () => hideRom(r) },
    ]);
  }
  async function dlPlay(d) {
    if (d.st !== "done") return;
    try {
      const r = await emuDetect(d.rom || d.path);
      if (r.emu) return playGame(r);
      await openPath(d.path);
    } catch (e) {
      flash(String(e));
    }
  }
  function dlCardAction(d) {
    return (name) => {
      if (name === "reveal") return reveal(d.path, d.dir).catch((e) => flash(String(e)));
      if (name === "pause" || name === "resume") downloadAction([d.id], name).then(refreshDownloads).catch((e) => flash(String(e)));
    };
  }
  async function dlExtractCard(d) {
    try {
      const rom = await dlExtract(d.id);
      flash(`${rom.name} extrait · zip mis à la corbeille`);
      refreshDownloads();
    } catch (e) {
      flash(String(e));
    }
  }
  function dlCardMenu(e, g) {
    const d = g.dl;
    const zip = /\.(zip|7z)$/i.test(d.name);
    openMenu(e, [
      d.st === "done" && { label: "Jouer", icon: "play", run: () => dlPlay(d) },
      d.st === "done" && zip && (d.rom ? { label: "Supprimer le zip (déjà extrait)", icon: "trash", run: () => dlExtractCard(d) } : { label: "Extraire le jeu", icon: "layers", run: () => dlExtractCard(d) }),
      { label: "Vrai nom du jeu…", icon: "tag", run: () => renameGame(g) },
      { label: "Choisir l'image…", icon: "image", run: () => pickArt(g) },
      { label: "Ouvrir le dossier", icon: "folder", run: () => reveal(d.path, d.dir).catch((err) => flash(String(err))) },
      "-",
      { label: "Voir dans Téléchargements", icon: "download", run: () => (view = "downloads") },
    ]);
  }
  const dlActive = $derived(downloads.filter((d) => d.st === "running" || d.st === "queued" || d.st === "paused").length);
  let categories = $state([]);
  let disks = $state([]);
  let loaded = $state(false);
  let error = $state(null);
  let query = $state("");
  // Lien collé dans la recherche → onglet DL, prêt à télécharger
  let dlPrefill = $state(null);
  function pasteSearch(e) {
    const text = e.clipboardData?.getData("text")?.trim() ?? "";
    if (!/^https?:\/\/\S+(\s+https?:\/\/\S+)*$/i.test(text)) return;
    e.preventDefault();
    dlPrefill = { text, at: Date.now() };
    view = "downloads";
    query = "";
  }
  let selectedKey = $state(null); // groupe ouvert dans le panneau
  let sel = $state(new Set()); // sélection multiple
  let settingsOpen = $state(false);
  let settingsTab = $state("general");
  let compact = $state(false);
  let addOpen = $state(false);
  let addInit = $state(null); // { query, source, prefer, code } pour une recherche préremplie
  let globalPause = $state(false); // bandeau seulement après « Tout mettre en pause »
  let dragging = $state(false);
  const THEMES_IDS = THEMES.map((t) => t.id);
  let searchEl, mainEl;
  // Recherche absente de la bibliothèque → proposer de la chercher sur les sites (modale +)
  let searchFocus = $state(false);
  const webQuery = $derived(query.trim().length >= 2 && !/^(magnet:|https?:)/i.test(query.trim()) ? query.trim() : "");
  function searchWeb(q = webQuery) {
    if (!q) return;
    addInit = { query: q, kind: prefs.kind !== "all" ? prefs.kind : null };
    addOpen = true;
    query = "";
    searchEl?.blur();
  }
  function searchKey(e) {
    if (e.key !== "Enter" || !webQuery) return;
    if (e.shiftKey || !searched.length) (e.preventDefault(), searchWeb());
  }

  // Torrents ajoutés sans démarrer, en attente du choix des fichiers (un à la fois)
  let staged = $state([]);
  let pickHidden = $state(false);
  function onStaged(res) {
    if (res.errors?.length) flash(res.errors[0]);
    if (res.direct?.length) {
      // Direction l'onglet DL, où le téléchargement démarre
      addOpen = false;
      addInit = null;
      view = "downloads";
      refreshDownloads();
    }
    if (res.hashes?.length) staged = [...staged, ...res.hashes.filter((h) => !staged.includes(h))];
    refresh();
  }

  // ROM double-cliquée dont l'émulateur n'est pas prêt → fenêtre « Jouer »
  async function takeRoms() {
    const r = await emuTakePending().catch(() => []);
    if (r.length) ui.roms = [...ui.roms, ...r.filter((x) => !ui.roms.includes(x))];
  }
  $effect(() => {
    let un;
    takeRoms();
    listen("rom-open", takeRoms).then((f) => (un = f));
    return () => un?.();
  });

  // Fichier récupéré sur un hébergeur (fenêtre qui fait les clics) : direction l'onglet DL
  $effect(() => {
    let un;
    listen("host-download", () => {
      addOpen = false;
      addInit = null;
      view = "downloads";
      refreshDownloads();
    }).then((f) => (un = f));
    return () => un?.();
  });

  // .torrent ouvert depuis l'Explorateur / lien magnet cliqué
  $effect(() => {
    let un;
    listen("torrents-staged", (ev) => onStaged(ev.payload)).then((f) => (un = f));
    return () => un?.();
  });

  // Glisser-déposer de fichiers .torrent n'importe où sur la fenêtre
  $effect(() => {
    let un;
    getCurrentWebview()
      .onDragDropEvent(async (ev) => {
        const p = ev.payload;
        if (p.type === "enter" || p.type === "over") dragging = (p.paths ?? []).some((x) => x.toLowerCase().endsWith(".torrent")) || dragging;
        else if (p.type === "leave") dragging = false;
        else if (p.type === "drop") {
          dragging = false;
          const paths = p.paths.filter((x) => x.toLowerCase().endsWith(".torrent"));
          if (!paths.length) return;
          try {
            onStaged(await stageTorrents([], paths));
          } catch (e) {
            flash(String(e));
          }
        }
      })
      .then((f) => (un = f));
    return () => un?.();
  });

  const win = getCurrentWindow();

  // ---------- thème & mode compact ----------
  $effect(() => {
    if (!THEMES_IDS.includes(prefs.theme)) prefs.theme = "light";
    document.documentElement.dataset.theme = prefs.theme;
    win.setTheme(themeOf(prefs.theme).dark ? "dark" : "light").catch(() => {});
  });
  $effect(() => {
    const check = async () => {
      try {
        compact = !((await win.isMaximized()) || (await win.isFullscreen()));
      } catch {}
    };
    check();
    let un;
    win.onResized(check).then((f) => (un = f));
    return () => un?.();
  });
  $effect(() => document.documentElement.classList.toggle("compact", compact));

  // ---------- données ----------
  let actionStamp = 0; // une réponse partie avant la dernière action est ignorée (pas de « retour en arrière »)
  // Pause/reprise demandées : on garde l'état voulu affiché tant que qBittorrent ne l'a pas confirmé (8 s max)
  const pending = new Map(); // hash | "all" → { pause: bool, until }
  function applyPending(list) {
    const now = Date.now();
    const all = pending.get("all");
    if (all && now > all.until) pending.delete("all");
    return list.map((t) => {
      const p = pending.get(t.hash) ?? pending.get("all");
      if (!p || t.status === "error") return t;
      if (now > p.until) {
        pending.delete(t.hash);
        return t;
      }
      const ok = p.pause === isPaused(t.status);
      if (ok) {
        pending.delete(t.hash);
        return t;
      }
      const done = t.progress >= 1;
      return p.pause
        ? { ...t, status: done ? "done" : "paused", dlspeed: 0, upspeed: 0 }
        : { ...t, status: done ? "seeding" : "downloading" };
    });
  }
  // Au lancement, qBittorrent démarre en arrière-plan : on affiche un écran de chargement
  // (et pas une erreur) tant qu'il n'a pas répondu, pendant 25 s maximum.
  const bootStart = Date.now();
  let booting = $state(true);

  async function refresh() {
    const started = Date.now();
    refreshDownloads();
    try {
      const t = await getTorrents();
      if (started < actionStamp) return;
      torrents = applyPending(t);
      error = null;
      if (booting) {
        booting = false;
        loadCategories();
      }
    } catch (e) {
      error = String(e);
      if (booting && Date.now() - bootStart > 25000) booting = false;
    } finally {
      loaded = true;
    }
  }

  // Pendant le démarrage, on réessaie souvent pour afficher la liste dès que possible
  $effect(() => {
    if (!booting) return;
    const id = setInterval(refresh, 600);
    return () => clearInterval(id);
  });

  const loadCategories = () => getCategories().then((c) => (categories = c)).catch(() => {});
  const loadDisks = () => getDisks().then((d) => (disks = d)).catch(() => {});

  $effect(() => {
    refresh();
    loadCategories();
    loadDisks();
    const id = setInterval(() => !document.hidden && refresh(), 2000);
    const id2 = setInterval(() => !document.hidden && loadDisks(), 30000);
    const onVis = () => !document.hidden && refresh();
    document.addEventListener("visibilitychange", onVis);
    return () => {
      clearInterval(id);
      clearInterval(id2);
      document.removeEventListener("visibilitychange", onVis);
    };
  });

  const matchStatus = (s, f) =>
    f === "all" || (f === "active" && isActive(s)) || (f === "done" && isDone(s)) || (f === "error" && s === "error");

  const searched = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return torrents;
    return torrents.filter((t) => t.title.toLowerCase().includes(q) || t.name.toLowerCase().includes(q));
  });
  const kindCounts = $derived.by(() => {
    const c = {};
    for (const t of searched) if (matchStatus(t.status, prefs.status)) c[t.kind] = (c[t.kind] ?? 0) + 1;
    c.all = Object.values(c).reduce((a, b) => a + b, 0);
    // + les jeux téléchargés hors torrent (affichés dans l'onglet Jeux)
    const q = query.trim().toLowerCase();
    const games = downloads.filter((d) => d.kind === "game" && matchStatus(DL_ST[d.st] ?? "done", prefs.status) && (!q || d.title.toLowerCase().includes(q))).length;
    const roms = matchStatus("done", prefs.status) ? romCards(q).length : 0;
    if (games + roms) c.game = (c.game ?? 0) + games + roms;
    return c;
  });
  const statusCounts = $derived.by(() => {
    const c = { all: 0, active: 0, done: 0, error: 0 };
    for (const t of searched) {
      if (prefs.kind !== "all" && t.kind !== prefs.kind) continue;
      for (const f of STATUS_FILTERS) if (matchStatus(t.status, f.id)) c[f.id]++;
    }
    return c;
  });
  const visible = $derived.by(() => {
    const items = searched.filter((t) => (prefs.kind === "all" || t.kind === prefs.kind) && matchStatus(t.status, prefs.status));
    return sortGroups(buildGroups(items, true), prefs.sort, prefs.dir);
  });
  const selected = $derived.by(() => {
    if (!selectedKey) return null;
    return buildGroups(torrents, true).find((g) => g.key === selectedKey) ?? null;
  });
  const selItems = $derived(visible.filter((g) => sel.has(g.key)).flatMap((g) => g.items));
  const totals = $derived(torrents.reduce((a, t) => ({ dl: a.dl + t.dlspeed, up: a.up + t.upspeed }), { dl: downloads.reduce((s, d) => s + d.speed, 0), up: 0 }));
  const allPaused = $derived(torrents.length > 0 && torrents.every((t) => isPaused(t.status) || t.status === "error"));

  // Disques utilisés par les torrents
  const drive = (p) => (p?.match(/^[a-z]:/i)?.[0] ?? "").toUpperCase();
  const usedDisks = $derived.by(() => {
    const used = new Set(torrents.map((t) => drive(t.save_path)).filter(Boolean));
    return disks.filter((d) => used.has(drive(d.mount)));
  });

  // ---------- actions ----------
  function optimistic(hashes, action) {
    if (action !== "pause" && action !== "resume") return;
    const until = Date.now() + 8000;
    if (hashes[0] === "all") {
      pending.clear();
      pending.set("all", { pause: action === "pause", until });
    } else {
      for (const h of hashes) pending.set(h, { pause: action === "pause", until });
    }
    actionStamp = Date.now();
    torrents = applyPending(torrents);
  }

  async function onaction(action, target) {
    try {
      if (action === "reveal") return await reveal(target.content_path || target.save_path, target.save_path);
      if (action === "play") return await startPlay(target);
      if (action === "share") return await share(target);
      if (action === "move") return await moveTo(target);
      if (action === "searchEpisode") {
        // Recherche préremplie pour un épisode sorti (depuis le calendrier d'une série)
        addInit = target;
        addOpen = true;
        return;
      }
      if (action === "removeMenu") {
        // Supprimer le « parent » = supprimer tous les torrents du groupe
        const { e, items } = target;
        return openMenu(e, [
          { label: items.length > 1 ? `Retirer les ${items.length} torrents` : "Retirer de qBittorrent", icon: "close", run: () => remove(items, false) },
          { label: "Supprimer avec les fichiers", icon: "trash", danger: true, run: () => remove(items, true) },
        ]);
      }
      if (action === "openInstall") return await openPath(target.location);
      if (action === "uninstall") return await uninstall(target);
      if (action === "deleteInstaller") return await deleteInstaller(target);
      if (action === "setup") {
        await runSetup(target.hash);
        return flash("Installation lancée");
      }
      const hashes = target.map((t) => t.hash);
      optimistic(hashes, action);
      await torrentAction(hashes, action);
      await refresh();
    } catch (e) {
      flash(String(e));
      refresh();
    }
  }

  async function startPlay(items) {
    try {
      const n = await play(items.map((i) => i.hash));
      flash(n > 1 ? `Lecture de ${n} fichiers à la suite` : "Lecture lancée");
    } catch (e) {
      flash(String(e));
    }
  }

  /** Action du double-clic : lire un film, ouvrir un livre, ouvrir le dossier d'un jeu… ; une série ouvre ses épisodes. */
  function primary(g) {
    const one = g.items.length === 1 ? g.items[0] : null;
    if (one && g.kind === "film" && one.progress > 0) return startPlay(g.items);
    if (one && g.kind === "book") return openPath(one.content_path).catch((e) => flash(String(e)));
    const inst = g.items.find((i) => i.installed)?.installed;
    if (inst?.location) return openPath(inst.location).catch((e) => flash(String(e)));
    if (g.kind === "game" || g.kind === "software" || g.kind === "other") return onaction("reveal", g.items[0]);
    selectedKey = g.key;
  }

  async function moveTo(items) {
    const dir = await pickFolder({ directory: true, title: `Déplacer ${items.length > 1 ? `${items.length} torrents` : `« ${items[0].title} »`} vers…`, defaultPath: items[0].save_path });
    if (typeof dir !== "string") return;
    await setLocation(items.map((i) => i.hash), dir);
    flash(`Déplacement vers ${dir} — qBittorrent copie les fichiers`);
    refresh();
  }

  /** Range une série (ou toutes, si `key` est absent) dans son propre dossier. */
  async function organize(key = null) {
    try {
      const n = await organizeShows(key);
      flash(n ? `${n} épisode${n > 1 ? "s" : ""} rangé${n > 1 ? "s" : ""} dans le dossier de leur série` : "Déjà rangé ✓");
      refresh();
    } catch (e) {
      flash(String(e));
    }
  }

  async function uninstall(prog) {
    const ok = await confirmAsk(`Désinstaller « ${prog.name} » ?`, "Le désinstalleur du programme va s'ouvrir.", "Désinstaller", true);
    if (!ok) return;
    await uninstallProgram(prog.name);
    flash("Désinstalleur lancé");
  }

  async function deleteInstaller(items) {
    const size = fmtSize(items.reduce((s, i) => s + i.completed, 0));
    const ok = await confirmAsk(
      "Supprimer l'installable ?",
      `Le torrent et ses fichiers d'installation (${size}) seront effacés. Le jeu ou logiciel installé n'est pas touché.`,
      "Supprimer l'installable",
      true,
    );
    if (!ok) return;
    await torrentAction(items.map((i) => i.hash), "delete_files");
    if (selected && items.length === selected.items.length) selectedKey = null;
    flash(`Installable supprimé (${size} libérés)`);
    refresh();
  }

  async function share(items) {
    try {
      const n = await shareTorrents(items.map((i) => i.hash));
      flash(`${n > 1 ? `${n} fichiers .torrent copiés` : "Fichier .torrent copié"} — colle-le où tu veux (Ctrl+V)`);
    } catch (e) {
      flash(String(e));
    }
  }

  async function toggleAll() {
    const action = allPaused ? "resume" : "pause";
    globalPause = action === "pause";
    try {
      optimistic(["all"], action);
      await torrentAction(["all"], action);
      flash(action === "resume" ? "Tout reprend" : "Tout est en pause (téléchargements et partage)");
      await refresh();
    } catch (e) {
      flash(String(e));
    }
  }

  async function remove(items, withFiles) {
    if (!items.length) return;
    const n = items.length;
    const size = fmtSize(items.reduce((s, i) => s + i.completed, 0));
    const what = n > 1 ? `${n} torrents` : `« ${items[0].title} »`;
    const ok = await confirmAsk(
      withFiles ? `Supprimer ${what} et leurs fichiers ?` : `Retirer ${what} de qBittorrent ?`,
      withFiles ? `${size} seront définitivement effacés du disque.` : "Les fichiers restent sur le disque.",
      withFiles ? "Supprimer définitivement" : "Retirer",
      withFiles,
    );
    if (!ok) return;
    await onaction(withFiles ? "delete_files" : "delete", items);
    const gone = new Set(items.map((i) => i.hash));
    if (selected && selected.items.every((i) => gone.has(i.hash))) selectedKey = null;
    sel = new Set();
    flash(withFiles ? "Supprimé, fichiers compris" : "Retiré de qBittorrent");
  }

  async function changeCategory(items, cat) {
    try {
      await setCategory(items.map((i) => i.hash), cat);
      flash(cat ? `Catégorie « ${cat} » — qBittorrent déplace les fichiers si besoin` : "Catégorie retirée");
      await refresh();
    } catch (e) {
      flash(String(e));
    }
  }

  const categoryMenu = (its) => ({
    label: "Catégorie",
    icon: "tag",
    sub: [
      { label: "Aucune", checked: its.every((i) => !i.category), run: () => changeCategory(its, "") },
      ...categories.map((c) => ({ label: c.name, checked: its.every((i) => i.category === c.name), run: () => changeCategory(its, c.name) })),
    ],
  });
  const shareMenu = (its, one) => ({
    label: "Partager",
    icon: "share",
    sub: [
      { label: "Copier le fichier .torrent", run: () => share(its) },
      one && { label: "Copier le lien magnet", run: () => copy(one.magnet) },
    ].filter(Boolean),
  });

  function menuFor(e, g, extra = []) {
    loadCategories();
    // Clic droit sur un élément de la sélection → menu pour toute la sélection
    if (sel.size > 1 && sel.has(g.key)) return multiMenu(e, selItems);
    const its = g.items;
    const one = its.length === 1 ? its[0] : null;
    const incomplete = its.filter((i) => i.progress < 1 && i.status !== "error");
    const running = incomplete.some((i) => !isPaused(i.status));
    const anyPaused = incomplete.some((i) => isPaused(i.status));
    // « Lire » : un film, ou un épisode isolé (pas un livre audio rempli de fichiers)
    const canPlay = one && (g.kind === "film" || ((g.kind === "series" || g.kind === "anime") && one.episode != null)) && one.progress > 0;
    const inst = its.find((i) => i.installed)?.installed;
    const installable = g.kind === "game" || g.kind === "software";
    // Studio d'extraits : un film / un épisode (fichier vidéo, ou dossier qui en contient)
    const clipable = one && one.progress > 0 && (canPlay || /\.(mkv|mp4|avi|m4v|mov|webm|ts|m2ts|wmv)$/i.test(one.content_path ?? ""));
    openMenu(e, [
      canPlay && { label: "Lire", icon: "play", run: () => startPlay(its) },
      clipable && { label: "Capturer un extrait…", icon: "scissors", run: () => openStudio(one.content_path || one.save_path) },
      ...extra,
      { label: "Afficher les détails", icon: "list", run: () => (selectedKey = g.key) },
      { label: inst ? "Ouvrir le dossier de l'installable" : "Ouvrir le dossier", icon: "folder", run: () => onaction("reveal", its[0]) },
      inst?.location && { label: "Ouvrir le dossier du jeu / logiciel installé", icon: "open", run: () => onaction("openInstall", inst) },
      inst && { label: "Désinstaller…", icon: "close", run: () => uninstall(inst) },
      inst && { label: "Supprimer l'installable", icon: "trash", danger: true, run: () => deleteInstaller(its) },
      installable && !inst && one && one.progress >= 1 && { label: "Installer", icon: "down", run: () => onaction("setup", one) },
      "-",
      running && { label: one ? "Pause" : "Tout mettre en pause", icon: "pause", run: () => onaction("pause", incomplete) },
      anyPaused && { label: one ? "Reprendre" : "Tout reprendre", icon: "play", run: () => onaction("resume", incomplete) },
      { label: "Revérifier", icon: "refresh", run: () => onaction("recheck", its) },
      { label: "Déplacer…", icon: "move", run: () => moveTo(its) },
      (g.kind === "series" || g.kind === "anime") && its.length > 1 && {
        label: "Ranger dans un dossier de série",
        icon: "folder",
        run: () => organize(g.artKey),
      },
      categoryMenu(its),
      shareMenu(its, one),
      "-",
      { label: "Copier le nom", icon: "copy", run: () => copy(one ? one.name : g.title) },
      { label: "Choisir l'image…", icon: "image", run: () => pickArt(g) },
      "-",
      { label: "Retirer de qBittorrent", icon: "close", run: () => remove(its, false) },
      { label: "Supprimer avec les fichiers", icon: "trash", danger: true, run: () => remove(its, true) },
    ]);
  }

  function multiMenu(e, its) {
    openMenu(e, [
      { label: `${its.length} torrents sélectionnés`, icon: "select", disabled: true },
      "-",
      { label: "Tout mettre en pause", icon: "pause", run: () => onaction("pause", its) },
      { label: "Tout reprendre", icon: "play", run: () => onaction("resume", its) },
      { label: "Revérifier", icon: "refresh", run: () => onaction("recheck", its) },
      { label: "Déplacer…", icon: "move", run: () => moveTo(its) },
      categoryMenu(its),
      shareMenu(its, null),
      "-",
      { label: "Retirer de qBittorrent", icon: "close", run: () => remove(its, false) },
      { label: "Supprimer avec les fichiers", icon: "trash", danger: true, run: () => remove(its, true) },
    ]);
  }

  const itemMenu = (e, it, g = null, extra = [], episode = false) => (episode ? episodeMenu(e, it, extra) : menuFor(e, g ?? buildGroups([it], false)[0], extra));

  // Épisode d'une série faite de plusieurs torrents : le même menu qu'un épisode d'un pack (pas de « détails »,
  // catégorie, déplacement…), plus partager et supprimer.
  function episodeMenu(e, it, watch = []) {
    const incomplete = it.progress < 1 && it.status !== "error";
    openMenu(e, [
      { label: "Lire", icon: "play", run: () => startPlay([it]), disabled: it.progress <= 0 },
      incomplete && !isPaused(it.status) && { label: "Pause", icon: "pause", run: () => onaction("pause", [it]) },
      incomplete && isPaused(it.status) && { label: "Reprendre", icon: "play", run: () => onaction("resume", [it]) },
      watch.length && "-",
      ...watch,
      it.progress > 0 && { label: "Capturer un extrait…", icon: "scissors", run: () => openStudio(it.content_path || it.save_path) },
      "-",
      shareMenu([it], it),
      { label: "Supprimer l'épisode", icon: "trash", danger: true, run: () => remove([it], true) },
    ]);
  }

  // Un clic ailleurs que sur une carte (ou sur les barres d'action) annule la sélection
  function clearSelOutside(e) {
    if (!sel.size || e.button !== 0 || e.ctrlKey || e.shiftKey) return;
    if (e.target.closest(".card, .selbar, .menu, .drawer, .modal, .dialog, .toast, button, input, select, label")) return;
    sel = new Set();
  }

  // ---------- sélection ----------
  let anchor = null;
  let clickTimer;

  function cardClick(e, g, i) {
    if (e.ctrlKey || e.metaKey) {
      const s = new Set(sel);
      s.has(g.key) ? s.delete(g.key) : s.add(g.key);
      sel = s;
      anchor = i;
      return;
    }
    if (e.shiftKey && anchor != null) {
      const [a, b] = anchor < i ? [anchor, i] : [i, anchor];
      sel = new Set([...sel, ...visible.slice(a, b + 1).map((x) => x.key)]);
      return;
    }
    anchor = i;
    if (sel.size) sel = new Set();
    // On attend un peu : un double-clic lance la lecture au lieu d'ouvrir le panneau.
    clearTimeout(clickTimer);
    clickTimer = setTimeout(() => (selectedKey = g.key), 220);
  }

  // Cartes hors torrent : Ctrl+clic pour sélectionner (pas de panneau de détails)
  function extraClick(e, g) {
    if (e.ctrlKey || e.metaKey || e.shiftKey) {
      const s = new Set(sel);
      s.has(g.key) ? s.delete(g.key) : s.add(g.key);
      sel = s;
    } else if (sel.size) sel = new Set();
  }
  const selExtra = $derived(dlCards.filter((g) => sel.has(g.key)));
  function hideSelected() {
    for (const g of selExtra) g.rom ? hideRom(g.rom) : downloadAction([g.dl.id], "delete").catch(() => {});
    sel = new Set();
    refreshDownloads();
  }
  async function trashSelected() {
    const n = selExtra.length;
    const size = selExtra.reduce((a, g) => a + (g.size || 0), 0);
    if (!(await confirmAsk(`Supprimer ${n} jeu${n > 1 ? "x" : ""} ?`, `${fmtSize(size)} mis à la corbeille.`, "Supprimer", true))) return;
    for (const g of selExtra) {
      try {
        if (g.rom) {
          await trashPath(g.rom.path);
          romLib = romLib.filter((r) => r.path !== g.rom.path);
        } else await downloadAction([g.dl.id], "delete_files");
      } catch (e) {
        flash(String(e));
      }
    }
    sel = new Set();
    refreshDownloads();
  }

  function cardDbl(e, g) {
    clearTimeout(clickTimer);
    if (e.ctrlKey || e.shiftKey || e.metaKey) return;
    primary(g);
  }

  // Cliquer-glisser dans le vide → rectangle de sélection
  let band = $state(null);
  let bandBase = new Set();
  function bandDown(e) {
    // Onglet DL : pas de sélection par rectangle (la capture du pointeur volait les clics et doubles-clics des lignes)
    if (view === "downloads" || e.button !== 0 || e.target.closest(".card, button, input, select, a, .dls")) return;
    bandBase = e.ctrlKey || e.shiftKey ? new Set(sel) : new Set();
    band = { x0: e.clientX, y0: e.clientY, x1: e.clientX, y1: e.clientY, moved: false };
    mainEl.setPointerCapture(e.pointerId);
  }
  function bandMove(e) {
    if (!band) return;
    band.x1 = e.clientX;
    band.y1 = e.clientY;
    if (!band.moved && Math.hypot(band.x1 - band.x0, band.y1 - band.y0) < 5) return;
    band.moved = true;
    const [l, r] = [Math.min(band.x0, band.x1), Math.max(band.x0, band.x1)];
    const [t, b] = [Math.min(band.y0, band.y1), Math.max(band.y0, band.y1)];
    const s = new Set(bandBase);
    for (const el of mainEl.querySelectorAll("[data-key]")) {
      const rc = el.getBoundingClientRect();
      if (rc.right > l && rc.left < r && rc.bottom > t && rc.top < b) s.add(el.dataset.key);
    }
    sel = s;
  }
  function bandUp() {
    if (band && !band.moved) sel = new Set();
    band = null;
  }

  function onkey(e) {
    if (ui.confirm || ui.menu) return;
    const typing = e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement;
    if ((e.key === "/" && !typing) || (e.key.toLowerCase() === "k" && e.ctrlKey)) {
      e.preventDefault();
      searchEl?.focus();
    } else if (e.key === "Escape") {
      if (settingsOpen) settingsOpen = false;
      else if (selectedKey) selectedKey = null;
      else if (sel.size) sel = new Set();
      else if (typing) (query = ""), e.target.blur();
    } else if (!typing && !settingsOpen && !selectedKey && e.ctrlKey && e.key.toLowerCase() === "a") {
      e.preventDefault();
      sel = new Set(visible.map((g) => g.key));
    } else if (!typing && !settingsOpen && e.key === "Delete" && selItems.length) {
      remove(selItems, e.shiftKey);
    } else if (!typing && !settingsOpen && /^[1-9]$/.test(e.key) && KINDS[Number(e.key) - 1]) {
      prefs.kind = KINDS[Number(e.key) - 1].id;
    }
  }
</script>

<svelte:window onkeydown={onkey} onpointerdown={clearSelOutside} oncontextmenu={(e) => !(e.target instanceof HTMLInputElement) && e.preventDefault()} />

<div class="app">
  <header class="top">
    <div class="brand">
      <img class="logo" src={customLogo ?? logo} alt="OneForAll" />
      <span class="brand-name">
        <b>OneForAll</b>
        <small>TORRENT CLIENT{#if upd.current} <em>v{upd.current}</em>{/if}</small>
      </span>
    </div>

    <button class="add" title="Ajouter un torrent (rechercher, magnet, fichier)" onclick={() => (addOpen = true)}><Icon name="plus" size={18} /></button>

    <div class="search-wrap">
      <label class="search">
        <Icon name="search" size={16} />
        <input bind:this={searchEl} bind:value={query} onpaste={pasteSearch} onkeydown={searchKey} onfocus={() => (searchFocus = true)} onblur={() => (searchFocus = false)} placeholder="Rechercher… ou colle un lien à télécharger" spellcheck="false" />
        <kbd>/</kbd>
      </label>
      {#if searchFocus && webQuery}
        <div class="suggest" transition:fly={{ y: -6, duration: 120 }}>
          <p class="sg-local">{searched.length ? `${searched.length} torrent${searched.length > 1 ? "s" : ""} dans ta bibliothèque` : "Pas dans ta bibliothèque"}</p>
          <button class="sg-go" onmousedown={(e) => e.preventDefault()} onclick={() => searchWeb()}>
            <span class="sg-ic"><Icon name="download" size={16} /></span>
            <span class="sg-txt"><b>Chercher « {webQuery} » pour le télécharger</b><small>C411, Nyaa, Online-Fix, FitGirl, NXBrew</small></span>
            <kbd>{searched.length ? "Maj+Entrée" : "Entrée"}</kbd>
          </button>
        </div>
      {/if}
    </div>

    <div class="right">
      {#each usedDisks as d (d.mount)}
        {@const usedPct = ((d.total - d.free) / d.total) * 100}
        <button
          class="disk"
          class:low={d.free / d.total < 0.1}
          title="{drive(d.mount)} — {fmtSize(d.free)} libres sur {fmtSize(d.total)}"
          onclick={() => ((settingsTab = "storage"), (settingsOpen = true))}
        >
          <Icon name="drive" size={14} />
          <span class="dl">{drive(d.mount)}</span>
          <span class="dtext"><b>{fmtSize(d.free)}</b> / {fmtSize(d.total)}</span>
          <span class="dbar"><span style="width:{usedPct}%"></span></span>
        </button>
      {/each}
      <div class="speeds" title={error ?? "Connecté à qBittorrent"}>
        <span class="sp down"><Icon name="down" size={14} />{fmtSpeed(totals.dl)}</span>
        <span class="sp up"><Icon name="up" size={14} />{fmtSpeed(totals.up)}</span>
        <span class="conn" class:off={!!error}></span>
      </div>
      <button class="tbtn" class:warn={allPaused} onclick={toggleAll} title={allPaused ? "Tout reprendre" : "Tout mettre en pause (téléchargement et partage)"}>
        <Icon name={allPaused ? "playAll" : "pauseAll"} size={17} />
        <span class="tlabel">{allPaused ? "Tout reprendre" : "Tout en pause"}</span>
      </button>
      <button class="icon-btn" title={themeOf(prefs.theme).dark ? "Thème clair" : "Thème sombre"} onclick={() => (prefs.theme = themeOf(prefs.theme).dark ? "light" : "dark")}>
        <Icon name={themeOf(prefs.theme).dark ? "sun" : "moon"} size={16} />
      </button>
      <button class="icon-btn" title="Réglages" onclick={() => ((settingsTab = "general"), (settingsOpen = true))}>
        <Icon name="settings" size={16} />
      </button>
    </div>
  </header>

  {#if upd.available && !updDismissed}
    <div class="update-banner" transition:fly={{ y: -10, duration: 160 }}>
      <Icon name="down" size={15} />
      {#if upd.progress != null}
        <span>Installation de OneForAll {upd.available.version}… {upd.progress ? `${Math.round(upd.progress * 100)} %` : ""}</span>
        <div class="ub-bar"><span style="width:{(upd.progress ?? 0) * 100}%"></span></div>
      {:else}
        <span><b>OneForAll {upd.available.version}</b> est disponible (tu as la {upd.current}).</span>
        <button class="ub-go" onclick={installUpdate}>Mettre à jour</button>
        <button class="ub-x" title="Plus tard" onclick={() => (updDismissed = true)}><Icon name="close" size={14} /></button>
      {/if}
    </div>
  {/if}

  {#if globalPause && allPaused}
    <div class="paused-banner" transition:fly={{ y: -10, duration: 160 }}>
      <Icon name="pause" size={14} />Tout est en pause : rien ne se télécharge ni ne se partage.
      <button onclick={toggleAll}>Tout reprendre</button>
    </div>
  {/if}

  <nav class="filters">
    <div class="chips">
      {#each KINDS as k, i}
        {#if k.id === "all" || kindCounts[k.id] || prefs.kind === k.id}
          <button class="chip" class:on={view === "torrents" && prefs.kind === k.id} onclick={() => ((prefs.kind = k.id), (view = "torrents"))} title="{k.label} (raccourci : {i + 1})">
            <Icon name={k.id} size={15} />
            <span class="n">{kindCounts[k.id] ?? 0}</span>
          </button>
        {/if}
      {/each}
      <span class="vsep"></span>
      <button class="chip dlchip" class:on={view === "downloads"} onclick={() => (view = view === "downloads" ? "torrents" : "downloads")} title="Téléchargements directs (liens http)">
        <Icon name="download" size={15} />
        <span class="dltxt">DL</span>
        {#if dlActive}<span class="n">{dlActive}</span>{/if}
      </button>
    </div>

    <div class="tools" class:hidden={view === "downloads"}>
      <div class="seg">
        {#each STATUS_FILTERS as f}
          <button class:on={prefs.status === f.id} class:warn={f.id === "error" && statusCounts.error > 0} onclick={() => (prefs.status = f.id)}>
            {f.label}<span class="n">{statusCounts[f.id]}</span>
          </button>
        {/each}
      </div>
      <div class="sort">
        <select bind:value={prefs.sort} aria-label="Trier par">
          {#each SORTS as s}<option value={s.id}>{s.label}</option>{/each}
        </select>
        <button class="icon-btn" title={prefs.dir === "desc" ? "Décroissant" : "Croissant"} onclick={() => (prefs.dir = prefs.dir === "desc" ? "asc" : "desc")}>
          <Icon name={prefs.dir === "desc" ? "sortDesc" : "sortAsc"} size={16} />
        </button>
      </div>
    </div>
  </nav>

  <main bind:this={mainEl} onpointerdown={bandDown} onpointermove={bandMove} onpointerup={bandUp} onpointercancel={bandUp}>
    {#if view === "downloads"}
      <Downloads {downloads} onrefresh={refreshDownloads} prefill={dlPrefill} />
    {:else if booting && !torrents.length}
      <!-- Écran de démarrage : qBittorrent se lance en arrière-plan -->
      <div class="boot" in:fade={{ duration: 200 }}>
        <div class="boot-logo">
          <span class="pulse r1"></span>
          <span class="pulse r2"></span>
          <img class="core" src={logo} alt="" />
        </div>
        <h2>Démarrage du moteur…</h2>
        <p>qBittorrent se lance en arrière-plan</p>
        <div class="boot-bar"><span></span></div>
      </div>
    {:else if error && !torrents.length}
      <div class="empty connect" in:fade>
        {#if qstatus && !qstatus.installed}
          <Icon name="down" size={40} />
          <h2>qBittorrent n'est pas installé</h2>
          <p>OneForAll s'appuie sur qBittorrent pour télécharger (il tourne en arrière-plan, invisible). Installe-le puis relance OneForAll.</p>
          <div class="cbtns">
            <button class="btn primary" onclick={() => openUrl("https://www.qbittorrent.org/download")}><Icon name="open" size={15} />Télécharger qBittorrent</button>
            <button class="btn" onclick={refresh}><Icon name="refresh" size={15} />Réessayer</button>
          </div>
        {:else}
          <Icon name="link" size={40} />
          <h2>Relier qBittorrent à OneForAll</h2>
          <p>
            OneForAll parle à qBittorrent via son interface web, qui n'est pas encore activée.
            Il peut la configurer pour toi : accès réservé à ce PC, identifiants générés automatiquement.
            {#if qstatus?.running}<br /><b>qBittorrent va être redémarré</b> (tes téléchargements reprendront tout seuls).{/if}
          </p>
          <div class="cbtns">
            <button class="btn primary" disabled={settingUp} onclick={autoSetup}>
              {#if settingUp}<span class="spin"></span>Configuration…{:else}<Icon name="settings" size={15} />Configurer automatiquement{/if}
            </button>
            <button class="btn" disabled={settingUp} onclick={refresh}><Icon name="refresh" size={15} />Réessayer</button>
          </div>
          <small class="detail">{error}</small>
        {/if}
      </div>
    {:else if loaded && !visible.length && !dlCards.length && prefs.kind !== "game"}
      <div class="empty" in:fade>
        <Icon name="search" size={40} />
        <h2>Rien ici</h2>
        <p>Aucun élément ne correspond à ces filtres.</p>
        {#if webQuery}<button class="btn primary" onclick={() => searchWeb()}><Icon name="download" size={15} />Chercher « {webQuery} » pour le télécharger</button>{/if}
      </div>
    {:else}
      {#if visible.length}
      <div class="grid">
        {#each visible as g, i (g.key)}
          <div class="cell" animate:flip={{ duration: 280 }}>
            <Card
              group={g}
              sort={prefs.sort}
              selected={sel.has(g.key)}
              onclick={(e) => cardClick(e, g, i)}
              ondblclick={(e) => cardDbl(e, g)}
              oncontext={(e) => menuFor(e, g)}
              {onaction}
            />
          </div>
        {/each}
      </div>
      {/if}
      {#if prefs.kind === "game"}
        <section class="dlsec" in:fade>
          <h3>
            <span class="dlsec-ic"><Icon name="game" size={14} /></span>Hors torrent
            {#if dlCards.length}<span class="dlsec-n">{dlCards.length}</span>{/if}
            <span class="dlsec-sub">téléchargés en direct et ROM trouvées sur le PC</span>
            <button class="dlsec-scan" disabled={!!romScanning} onclick={scanRoms} title="Cherche sur tous tes disques les jeux DS, GBA, PSP, Switch… déjà téléchargés">
              {#if romScanning}<span class="spin dark"></span>{romScanning.files.toLocaleString("fr-FR")} fichiers · {romScanning.found} jeu{romScanning.found > 1 ? "x" : ""}{:else}<Icon name="search" size={14} />Scanner mes ROM{/if}
            </button>
          </h3>
          {#if !dlCards.length}
            <p class="dlsec-empty">Tes jeux DS, GBA, PSP, Switch… déjà sur le PC peuvent apparaître ici : clique sur « Scanner mes ROM ».</p>
          {/if}
          <div class="grid">
            {#each dlCards as g (g.key)}
              <div class="cell" animate:flip={{ duration: 280 }}>
                {#if g.rom}
                  <Card group={g} sort={prefs.sort} selected={sel.has(g.key)} onclick={(e) => extraClick(e, g)} ondblclick={() => romPlay(g.rom)} oncontext={(e) => romCardMenu(e, g)} onaction={(n) => n === "reveal" && reveal(g.rom.path, null).catch((e) => flash(String(e)))} />
                {:else}
                  <Card group={g} sort={prefs.sort} selected={sel.has(g.key)} onclick={(e) => extraClick(e, g)} ondblclick={() => dlPlay(g.dl)} oncontext={(e) => dlCardMenu(e, g)} onaction={dlCardAction(g.dl)} />
                {/if}
              </div>
            {/each}
          </div>
        </section>
      {/if}
    {/if}
  </main>
</div>

{#if band?.moved}
  <div
    class="band"
    style="left:{Math.min(band.x0, band.x1)}px;top:{Math.min(band.y0, band.y1)}px;width:{Math.abs(band.x1 - band.x0)}px;height:{Math.abs(band.y1 - band.y0)}px"
  ></div>
{/if}

{#if sel.size}
  <div class="selbar" transition:fly={{ y: 20, duration: 160 }}>
    <span class="count"><b>{sel.size}</b>sélectionné{sel.size > 1 ? "s" : ""}</span>
    {#if selItems.length}
    <button onclick={() => onaction("pause", selItems)}><Icon name="pause" size={14} />Pause</button>
    <button onclick={() => onaction("resume", selItems)}><Icon name="play" size={14} />Reprendre</button>
    <button onclick={(e) => { const r = e.currentTarget.getBoundingClientRect(); loadCategories(); openMenu({ preventDefault() {}, stopPropagation() {}, clientX: r.left, clientY: r.top - 8 }, categoryMenu(selItems).sub); }}><Icon name="tag" size={14} />Catégorie</button>
    <button onclick={() => moveTo(selItems)}><Icon name="move" size={14} />Déplacer</button>
    <button onclick={() => share(selItems)}><Icon name="share" size={14} />Partager</button>
    <button class="danger" onclick={() => remove(selItems, false)}><Icon name="close" size={14} />Retirer</button>
    <button class="danger" onclick={() => remove(selItems, true)}><Icon name="trash" size={14} />Supprimer</button>
    {:else}
    <button onclick={hideSelected}><Icon name="close" size={14} />Masquer</button>
    <button class="danger" onclick={trashSelected}><Icon name="trash" size={14} />Supprimer</button>
    {/if}
    <button class="x" title="Désélectionner (Échap)" onclick={() => (sel = new Set())}><Icon name="close" size={14} /></button>
  </div>
{/if}

{#if selected}
  <Drawer group={selected} onclose={() => (selectedKey = null)} {onaction} onitemmenu={itemMenu} />
{/if}

{#if addOpen}
  <AddTorrent {torrents} initial={addInit} onclose={() => ((addOpen = false), (addInit = null))} onadded={onStaged} />
{/if}

{#if staged.length && !pickHidden}
  {#key staged[0]}
    <PickFiles
      hash={staged[0]}
      {torrents}
      onclose={() => ((staged = staged.slice(1)), refresh())}
      onfreespace={() => ((pickHidden = true), (settingsTab = "storage"), (settingsOpen = true))}
    />
  {/key}
{:else if staged.length && pickHidden}
  <!-- Ajout mis de côté le temps de libérer de la place -->
  <button class="resume-add" onclick={() => ((pickHidden = false), (settingsOpen = false))}>
    <Icon name="down" size={15} />Reprendre l'ajout en attente
  </button>
{/if}

{#if dragging}
  <div class="dropzone" transition:fade={{ duration: 120 }}><Icon name="torrent" size={44} /><b>Dépose tes .torrent ici</b></div>
{/if}

{#if settingsOpen}
  <Settings {prefs} {torrents} bind:tab={settingsTab} onclose={() => (settingsOpen = false)} />
{/if}

<ContextMenu />
{#if ui.studio}
  <Studio path={ui.studio} onexported={() => ((view = "downloads"), refreshDownloads())} />
{/if}
{#if ui.roms.length}<Play />{/if}
{#if ui.artPick}<ArtPicker />{/if}
{#if ui.rename}<RenameGame />{/if}
{#if ui.diskMap}<DiskMap mount={ui.diskMap.mount} used={ui.diskMap.used} {torrents} {downloads} onclose={() => (ui.diskMap = null)} />{/if}
<Confirm />

{#if ui.toast}
  <div class="toast" transition:fade={{ duration: 150 }}>
    <span>{ui.toast.msg}</span>
    {#each ui.toast.actions as a}
      <button onclick={() => { a.run(); ui.toast = null; }}><Icon name={a.icon} size={14} />{a.label}</button>
    {/each}
  </div>
{/if}

<style>
  .app { display: flex; flex-direction: column; height: 100vh; }

  .top {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 10px var(--pad);
    border-bottom: 1px solid var(--line);
    background: var(--topbar);
    backdrop-filter: blur(14px);
  }
  .brand { display: flex; align-items: center; gap: 10px; flex: none; }
  .logo { width: 36px; height: 36px; border-radius: 9px; }
  /* Logo horizontal : nom en lettres condensées + « Torrent client » */
  .brand-name { display: flex; flex-direction: column; line-height: 1; }
  .brand-name b { font-family: Bahnschrift, "Segoe UI", sans-serif; font-weight: 700; font-size: 19px; letter-spacing: -0.01em; color: var(--text); }
  .brand-name small { margin-top: 3px; font-family: Bahnschrift, "Segoe UI", sans-serif; font-size: 9px; font-weight: 600; letter-spacing: 0.22em; color: var(--accent); white-space: nowrap; }
  .brand-name em { font-style: normal; letter-spacing: 0.05em; opacity: 0.75; }

  .resume-add { position: fixed; left: 20px; bottom: 20px; z-index: 130; display: inline-flex; align-items: center; gap: 8px; height: 42px; padding: 0 16px; border-radius: 999px; border: 0; background: var(--accent); color: white; font: inherit; font-size: 13px; font-weight: 700; cursor: pointer; box-shadow: var(--shadow-lg); animation: bob 2s ease-in-out infinite; }
  @keyframes bob { 50% { transform: translateY(-3px); } }
  .add { display: grid; place-items: center; width: 34px; height: 34px; border-radius: 10px; border: 0; background: var(--accent); color: white; cursor: pointer; flex: none; box-shadow: 0 4px 14px -4px var(--accent); transition: transform 0.12s, filter 0.12s; }
  .add:hover { filter: brightness(1.1); transform: scale(1.05); }
  .dropzone { position: fixed; inset: 12px; z-index: 400; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; border: 3px dashed var(--accent); border-radius: 20px; background: rgb(0 103 192 / 0.12); backdrop-filter: blur(4px); color: var(--accent); font-size: 18px; pointer-events: none; }
  .search-wrap { position: relative; flex: 1; max-width: 460px; min-width: 140px; margin: 0 auto; }
  .suggest { position: absolute; z-index: 80; top: calc(100% + 6px); left: 0; right: 0; padding: 6px; border-radius: 12px; border: 1px solid var(--line-strong); background: var(--menu-bg); backdrop-filter: blur(16px); box-shadow: var(--shadow-lg); }
  .sg-local { margin: 2px 8px 6px; font-size: 11.5px; color: var(--muted); }
  .sg-go { width: 100%; display: flex; align-items: center; gap: 10px; padding: 8px; border: 0; border-radius: 9px; background: var(--accent-soft); color: var(--text); font: inherit; text-align: left; cursor: pointer; }
  .sg-go:hover { background: var(--ring); }
  .sg-ic { flex: none; display: grid; place-items: center; width: 32px; height: 32px; border-radius: 8px; background: var(--accent); color: white; }
  .sg-txt { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .sg-txt b { font-size: 13px; font-weight: 650; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sg-txt small { font-size: 11px; color: var(--muted); }
  .search {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 12px;
    height: 34px;
    border-radius: 10px;
    background: var(--surface);
    border: 1px solid var(--line-strong);
    color: var(--muted);
    transition: border-color 0.15s, box-shadow 0.15s;
  }
  .search:focus-within { border-color: var(--accent); box-shadow: 0 0 0 3px var(--ring); }
  .search input { flex: 1; min-width: 0; background: none; border: 0; outline: none; color: var(--text); font: inherit; font-size: 14px; }
  kbd { font: 600 11px ui-monospace, monospace; padding: 2px 6px; border-radius: 5px; background: var(--chip); color: var(--muted); }

  .right { display: flex; align-items: center; gap: 8px; }
  .disk { display: grid; grid-template-columns: auto auto auto; grid-template-rows: auto 3px; column-gap: 6px; row-gap: 3px; align-items: center; height: 34px; padding: 4px 10px; border-radius: 10px; border: 0; background: var(--chip); color: var(--text-2); font: inherit; font-size: 12px; cursor: pointer; }
  .disk:hover { background: var(--accent-soft); }
  .disk .dl { font-weight: 800; color: var(--text); }
  .dtext { white-space: nowrap; font-variant-numeric: tabular-nums; }
  .dtext b { color: var(--text); font-weight: 700; }
  .dbar { grid-column: 1 / -1; height: 3px; border-radius: 3px; background: var(--line-strong); overflow: hidden; }
  .dbar span { display: block; height: 100%; background: var(--accent); }
  .disk.low .dbar span { background: var(--err); }
  .disk.low .dtext b { color: var(--err); }
  .speeds { display: flex; align-items: center; gap: 10px; padding: 0 12px; height: 34px; border-radius: 10px; background: var(--chip); font-size: 12.5px; font-weight: 600; font-variant-numeric: tabular-nums; white-space: nowrap; }
  .sp { display: inline-flex; align-items: center; gap: 4px; }
  .sp.down { color: var(--accent); }
  .sp.up { color: var(--ok); }

  /* ---------- écran de démarrage ---------- */
  .boot { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; height: 100%; min-height: 60vh; text-align: center; }
  .boot h2 { margin: 18px 0 0; font-size: 18px; }
  .boot p { margin: 0; color: var(--muted); font-size: 13px; }
  .boot-logo { position: relative; width: 84px; height: 84px; display: grid; place-items: center; }
  .boot-logo .core { position: relative; z-index: 1; width: 72px; height: 72px; border-radius: 18px; box-shadow: 0 10px 30px -10px var(--accent); animation: float 2.4s ease-in-out infinite; }
  .boot-logo .pulse { position: absolute; inset: 10px; border-radius: 22px; border: 2px solid var(--accent); opacity: 0; animation: ripple 2.4s ease-out infinite; }
  .boot-logo .r2 { animation-delay: 1.2s; }
  @keyframes ripple { 0% { transform: scale(0.9); opacity: 0.6; } 100% { transform: scale(1.7); opacity: 0; } }
  @keyframes float { 50% { transform: translateY(-4px); } }
  .boot-bar { width: 180px; height: 4px; margin-top: 14px; border-radius: 4px; background: var(--chip); overflow: hidden; }
  .boot-bar span { display: block; width: 40%; height: 100%; border-radius: 4px; background: linear-gradient(90deg, var(--accent), var(--accent-2)); animation: slide 1.1s ease-in-out infinite; }
  @keyframes slide { 0% { transform: translateX(-100%); } 100% { transform: translateX(250%); } }
  .conn { width: 7px; height: 7px; border-radius: 50%; background: var(--ok); box-shadow: 0 0 8px var(--ok); }
  .conn.off { background: var(--err); box-shadow: 0 0 8px var(--err); }
  .tbtn { display: inline-flex; align-items: center; gap: 7px; height: 34px; padding: 0 12px; border-radius: 10px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: inherit; font-size: 13px; font-weight: 600; cursor: pointer; white-space: nowrap; }
  .tbtn:hover { border-color: var(--accent); color: var(--accent); }
  .tbtn.warn { background: #f59e0b; border-color: #f59e0b; color: #1a1205; }

  .update-banner { display: flex; align-items: center; gap: 10px; padding: 8px var(--pad); background: var(--accent); color: white; font-size: 13px; }
  .ub-go { margin-left: auto; height: 28px; padding: 0 14px; border-radius: 7px; border: 0; background: white; color: var(--accent); font: inherit; font-size: 12.5px; font-weight: 800; cursor: pointer; }
  .ub-x { display: grid; place-items: center; width: 28px; height: 28px; border-radius: 7px; border: 0; background: rgb(255 255 255 / 0.15); color: white; cursor: pointer; }
  .ub-bar { flex: 1; max-width: 260px; height: 5px; margin-left: auto; border-radius: 5px; background: rgb(255 255 255 / 0.25); overflow: hidden; }
  .ub-bar span { display: block; height: 100%; background: white; transition: width 0.3s; }
  .paused-banner { display: flex; align-items: center; gap: 10px; padding: 8px var(--pad); background: #f59e0b; color: #1a1205; font-size: 13px; font-weight: 600; }
  .paused-banner button { margin-left: auto; height: 26px; padding: 0 12px; border-radius: 7px; border: 0; background: #1a1205; color: #fbbf24; font: inherit; font-size: 12px; font-weight: 700; cursor: pointer; }

  .filters { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 10px; padding: 12px var(--pad) 4px; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: var(--ctl-h);
    padding: 0 12px;
    border-radius: 999px;
    border: 1px solid var(--line-strong);
    background: var(--surface);
    color: var(--text-2);
    font: inherit;
    font-size: 13px;
    font-weight: 550;
    cursor: pointer;
    transition: all 0.15s;
  }
  .chip:hover { color: var(--text); border-color: var(--muted); }
  .chip.on { background: var(--accent-soft); color: var(--accent); border-color: var(--accent); box-shadow: 0 0 0 1px var(--accent) inset; }
  .chip.on .n { opacity: 1; }
  .n { font-size: 11px; font-weight: 700; opacity: 0.6; font-variant-numeric: tabular-nums; }

  .tools { display: flex; align-items: center; gap: 8px; }
  .tools.hidden { visibility: hidden; }
  .vsep { width: 1px; align-self: stretch; margin: 4px 4px; background: var(--line-strong); }
  .dlchip { gap: 6px; }
  .dltxt { font-size: 12px; font-weight: 700; letter-spacing: 0.03em; }
  .seg { display: flex; padding: 3px; border-radius: 10px; background: var(--chip); }
  .seg button { display: inline-flex; align-items: center; gap: 6px; height: calc(var(--ctl-h) - 6px); padding: 0 11px; border: 0; border-radius: 7px; background: none; color: var(--muted); font: inherit; font-size: 12.5px; font-weight: 550; cursor: pointer; }
  .seg button:hover { color: var(--text); }
  .seg button.on { background: var(--surface); color: var(--text); box-shadow: var(--shadow-sm); }
  .seg button.warn .n { color: var(--err); opacity: 1; }

  .sort { display: flex; align-items: center; gap: 4px; }
  select {
    height: var(--ctl-h);
    padding: 0 30px 0 12px;
    border-radius: 10px;
    border: 1px solid var(--line-strong);
    background: var(--surface) url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%238b90a0' stroke-width='2.5'%3E%3Cpath d='M6 9l6 6 6-6'/%3E%3C/svg%3E") no-repeat right 10px center;
    appearance: none;
    color: var(--text);
    font: inherit;
    font-size: 13px;
    cursor: pointer;
    outline: none;
  }
  select option { background: var(--bg-2); }
  .toggle { display: inline-flex; align-items: center; gap: 6px; height: var(--ctl-h); padding: 0 12px; border-radius: 10px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--muted); font: inherit; font-size: 13px; cursor: pointer; }
  .toggle.on { color: var(--accent); border-color: var(--accent); background: var(--accent-soft); }

  main { flex: 1; overflow-y: auto; padding: 16px var(--pad) 90px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(var(--card-min), 1fr)); gap: var(--gap-y) var(--gap-x); padding-top: 10px; }
  .cell { isolation: isolate; }
  .dlsec { margin-top: 26px; }
  .dlsec:first-child { margin-top: 0; }
  .dlsec h3 { display: flex; align-items: center; gap: 9px; margin: 0 0 4px; padding-top: 18px; border-top: 1px solid var(--line); font-size: 15px; font-weight: 700; color: var(--text); }
  .dlsec:first-child h3 { border-top: 0; padding-top: 10px; }
  .dlsec-ic { display: grid; place-items: center; width: 26px; height: 26px; border-radius: 8px; background: var(--accent-soft); color: var(--accent); }
  .dlsec-n { padding: 1px 8px; border-radius: 999px; background: var(--chip); color: var(--muted); font-size: 12px; font-weight: 650; }
  .dlsec-sub { font-size: 12px; font-weight: 500; color: var(--muted); }
  .spin.dark { border-color: var(--line-strong); border-top-color: var(--accent); }
  .dlsec-empty { margin: 10px 0 0; font-size: 13px; color: var(--muted); }
  .dlsec-scan { margin-left: auto; display: inline-flex; align-items: center; gap: 7px; height: 32px; padding: 0 14px; border-radius: 9px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: inherit; font-size: 13px; font-weight: 600; cursor: pointer; font-variant-numeric: tabular-nums; }
  .dlsec-scan:hover:not(:disabled) { border-color: var(--accent); color: var(--accent); }
  .dlsec-scan:disabled { cursor: default; color: var(--muted); }
  .band { position: fixed; z-index: 30; border: 1.5px solid var(--accent); background: var(--accent-soft); border-radius: 4px; pointer-events: none; }

  .selbar {
    position: fixed;
    left: 50%;
    bottom: 22px;
    translate: -50% 0;
    z-index: 35;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 6px 6px 6px 8px;
    border-radius: 18px;
    background: linear-gradient(135deg, color-mix(in srgb, var(--accent) 82%, white), var(--accent) 45%, color-mix(in srgb, var(--accent) 80%, black));
    color: white;
    box-shadow: 0 14px 34px -8px color-mix(in srgb, var(--accent) 60%, transparent), 0 2px 6px rgb(0 0 0 / 0.15), inset 0 1px 0 rgb(255 255 255 / 0.25);
  }
  .selbar .count { display: inline-flex; align-items: center; gap: 8px; padding: 0 12px 0 2px; margin-right: 4px; border-right: 1px solid rgb(255 255 255 / 0.25); font-weight: 700; font-size: 13px; white-space: nowrap; }
  .selbar .count b { display: grid; place-items: center; min-width: 26px; height: 26px; padding: 0 7px; border-radius: 13px; background: white; color: var(--accent); font-size: 13px; font-weight: 800; }
  .selbar button { display: inline-flex; align-items: center; gap: 6px; height: 34px; padding: 0 12px; border: 0; border-radius: 11px; background: none; color: white; font: inherit; font-size: 12.5px; font-weight: 600; cursor: pointer; white-space: nowrap; transition: background 0.12s; }
  .selbar button:hover { background: rgb(255 255 255 / 0.18); }
  .selbar button.danger { background: rgb(255 255 255 / 0.12); margin-left: 2px; }
  .selbar button.danger:hover { background: #e11d48; }
  .selbar .x { width: 34px; padding: 0; justify-content: center; margin-left: 4px; background: rgb(0 0 0 / 0.15); border-radius: 50%; }

  .empty { display: flex; flex-direction: column; align-items: center; gap: 10px; margin-top: 12vh; color: var(--muted); text-align: center; }
  .connect p { line-height: 1.55; max-width: 520px; }
  .cbtns { display: flex; gap: 8px; }
  .detail { margin-top: 10px; font-size: 11.5px; color: var(--muted); opacity: 0.7; }
  .spin { width: 14px; height: 14px; border-radius: 50%; border: 2px solid rgb(255 255 255 / 0.4); border-top-color: white; animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .empty h2 { margin: 6px 0 0; color: var(--text); font-size: 20px; }
  .empty p { margin: 0 0 8px; max-width: 460px; }

  .toast { position: fixed; bottom: 24px; left: 50%; transform: translateX(-50%); display: flex; align-items: center; gap: 8px; padding: 8px 8px 8px 16px; min-height: 40px; border-radius: 12px; background: var(--text); color: var(--on-text); font-size: 13px; font-weight: 600; z-index: 300; box-shadow: var(--shadow-lg); }
  .toast span { padding-right: 8px; }
  .toast button { display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 10px; border: 0; border-radius: 8px; background: rgb(127 127 127 / 0.25); color: inherit; font: inherit; font-size: 12px; font-weight: 700; cursor: pointer; }
  .toast button:hover { background: var(--accent); color: white; }

  /* Fenêtre étroite : on masque les libellés secondaires */
  :global(.compact) .tlabel { display: none; }
  @media (max-width: 1250px) { .tlabel, .dtext { display: none; } }
  @media (max-width: 900px) { .brand-name { display: none; } }
</style>
