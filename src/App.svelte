<script>
  import { flip } from "svelte/animate";
  import { fade, fly } from "svelte/transition";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import Card from "./lib/Card.svelte";
  import Drawer from "./lib/Drawer.svelte";
  import Settings from "./lib/Settings.svelte";
  import ContextMenu from "./lib/ContextMenu.svelte";
  import Confirm from "./lib/Confirm.svelte";
  import AddTorrent from "./lib/AddTorrent.svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { listen } from "@tauri-apps/api/event";
  import { stageTorrents, qbitStatus, setupQbit, openUrl, getCustomIcon, organizeShows } from "./lib/api.js";
  import PickFiles from "./lib/PickFiles.svelte";
  import Downloads from "./lib/Downloads.svelte";
  import { getDownloads } from "./lib/api.js";
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
  import { KINDS, PLAYABLE, SORTS, STATUS_FILTERS, buildGroups, fmtSize, fmtSpeed, isActive, isDone, isPaused, sortGroups } from "./lib/model.js";
  import { confirmAsk, copy, flash, openMenu, refreshArt, ui } from "./lib/ui.svelte.js";

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
  const dlActive = $derived(downloads.filter((d) => d.st === "running" || d.st === "queued" || d.st === "paused").length);
  let categories = $state([]);
  let disks = $state([]);
  let loaded = $state(false);
  let error = $state(null);
  let query = $state("");
  let selectedKey = $state(null); // groupe ouvert dans le panneau
  let sel = $state(new Set()); // sélection multiple
  let settingsOpen = $state(false);
  let settingsTab = $state("general");
  let compact = $state(false);
  let addOpen = $state(false);
  let addInit = $state(null); // { query, source, prefer, code } pour une recherche préremplie
  let globalPause = $state(false); // bandeau seulement après « Tout mettre en pause »
  let dragging = $state(false);
  let searchEl, mainEl;

  // Torrents ajoutés sans démarrer, en attente du choix des fichiers (un à la fois)
  let staged = $state([]);
  let pickHidden = $state(false);
  function onStaged(res) {
    if (res.errors?.length) flash(res.errors[0]);
    if (res.direct?.length) {
      flash(res.direct.length > 1 ? `${res.direct.length} liens directs ajoutés à l'onglet Téléchargements` : "Lien direct ajouté à l'onglet Téléchargements");
      refreshDownloads();
    }
    if (res.hashes?.length) staged = [...staged, ...res.hashes.filter((h) => !staged.includes(h))];
    refresh();
  }

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
    document.documentElement.dataset.theme = prefs.theme;
    win.setTheme(prefs.theme).catch(() => {});
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

  function menuFor(e, g) {
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
    openMenu(e, [
      canPlay && { label: "Lire", icon: "play", run: () => startPlay(its) },
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
      { label: "Actualiser l'image", icon: "image", run: () => refreshArt(g.artKey) },
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

  const itemMenu = (e, it, g = null) => menuFor(e, g ?? buildGroups([it], false)[0]);

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

  function cardDbl(e, g) {
    clearTimeout(clickTimer);
    if (e.ctrlKey || e.shiftKey || e.metaKey) return;
    primary(g);
  }

  // Cliquer-glisser dans le vide → rectangle de sélection
  let band = $state(null);
  let bandBase = new Set();
  function bandDown(e) {
    if (e.button !== 0 || e.target.closest(".card, button, input, select, a")) return;
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
    } else if (!typing && !settingsOpen && e.key === "Delete" && sel.size) {
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

    <label class="search">
      <Icon name="search" size={16} />
      <input bind:this={searchEl} bind:value={query} placeholder="Rechercher…" spellcheck="false" />
      <kbd>/</kbd>
    </label>

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
      <button class="icon-btn" title={prefs.theme === "light" ? "Thème sombre" : "Thème clair"} onclick={() => (prefs.theme = prefs.theme === "light" ? "dark" : "light")}>
        <Icon name={prefs.theme === "light" ? "moon" : "sun"} size={16} />
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
        <Icon name="link" size={15} />
        <span class="dltxt">Téléchargements</span>
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
      <Downloads {downloads} onrefresh={refreshDownloads} />
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
    {:else if loaded && !visible.length}
      <div class="empty" in:fade>
        <Icon name="search" size={40} />
        <h2>Rien ici</h2>
        <p>Aucun élément ne correspond à ces filtres.</p>
      </div>
    {:else}
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
    <span class="count"><Icon name="select" size={15} />{sel.size} sélectionné{sel.size > 1 ? "s" : ""}</span>
    <button onclick={() => onaction("pause", selItems)}><Icon name="pause" size={14} />Pause</button>
    <button onclick={() => onaction("resume", selItems)}><Icon name="play" size={14} />Reprendre</button>
    <button onclick={(e) => { const r = e.currentTarget.getBoundingClientRect(); loadCategories(); openMenu({ preventDefault() {}, stopPropagation() {}, clientX: r.left, clientY: r.top - 8 }, categoryMenu(selItems).sub); }}><Icon name="tag" size={14} />Catégorie</button>
    <button onclick={() => moveTo(selItems)}><Icon name="move" size={14} />Déplacer</button>
    <button onclick={() => share(selItems)}><Icon name="share" size={14} />Partager</button>
    <button class="danger" onclick={() => remove(selItems, false)}><Icon name="close" size={14} />Retirer</button>
    <button class="danger" onclick={() => remove(selItems, true)}><Icon name="trash" size={14} />Supprimer</button>
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
  .search {
    flex: 1;
    max-width: 460px;
    min-width: 140px;
    margin: 0 auto;
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
  .dlchip { gap: 7px; }
  .dltxt { font-size: 12.5px; font-weight: 650; }
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
  .band { position: fixed; z-index: 30; border: 1.5px solid var(--accent); background: var(--accent-soft); border-radius: 4px; pointer-events: none; }

  .selbar {
    position: fixed;
    left: 50%;
    bottom: 22px;
    translate: -50% 0;
    z-index: 35;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px;
    border-radius: 14px;
    background: var(--text);
    color: var(--on-text);
    box-shadow: var(--shadow-lg);
  }
  .selbar .count { display: inline-flex; align-items: center; gap: 7px; padding: 0 12px 0 8px; font-weight: 700; font-size: 13px; white-space: nowrap; }
  .selbar button { display: inline-flex; align-items: center; gap: 6px; height: 32px; padding: 0 11px; border: 0; border-radius: 9px; background: none; color: inherit; font: inherit; font-size: 12.5px; font-weight: 600; cursor: pointer; white-space: nowrap; }
  .selbar button:hover { background: rgb(127 127 127 / 0.25); }
  .selbar button.danger { color: #fb7185; }
  .selbar .x { width: 32px; padding: 0; justify-content: center; }

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
