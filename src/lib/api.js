import { invoke } from "@tauri-apps/api/core";
import { need } from "./need.svelte.js";

export const getTorrents = () => invoke("get_torrents");
export const torrentAction = (hashes, action) => invoke("torrent_action", { hashes, action });
export const getFiles = (hash) => invoke("get_files", { hash });
export const setFilePriority = (hash, ids, priority) => invoke("set_file_priority", { hash, ids, priority });
export const getCategories = () => invoke("get_categories");
export const setCategory = (hashes, category) => invoke("set_category", { hashes, category });
export const reveal = (path, fallback) => invoke("reveal", { path, fallback: fallback ?? "" });
export const getSettings = () => invoke("get_settings");
export const saveSettings = (settings) => invoke("save_settings", { settings });
export const getLimits = () => invoke("get_limits");
export const setLimits = (dl, up) => invoke("set_limits", { dl, up });
export const getDisks = () => invoke("get_disks");
export const scanTorrents = () => invoke("scan_torrents");
export const trashFiles = (paths) => invoke("trash_files", { paths });
export const getArtwork = (key, kind, title, year) => invoke("get_artwork", { key, kind, title, year: year ?? null });
export const forgetArtwork = (key) => invoke("forget_artwork", { key });
export const artChoices = (kind, title, year) => invoke("art_choices", { kind, title, year: year ?? null });
/** { url, fallback } d'une proposition, ou { path } d'un fichier, ou { data } (data: URL d'une image collée) */
export const setArtwork = (key, { url = null, fallback = null, path = null, data = null }) => invoke("set_artwork", { key, url, fallback, path, data });
// Pas de lecteur sur le PC : on propose de l’installer, puis on relance (renvoie null en attendant)
const withApp = (id, code, fn) => async (...a) => {
  try {
    return await fn(...a);
  } catch (e) {
    if (!String(e).includes(code)) throw e;
    need.app = id;
    need.retry = () => withApp(id, code, fn)(...a);
    return null;
  }
};
export const play = withApp("vlc", "NO_VLC", (hashes) => invoke("play", { hashes }));
export const shareTorrents = (hashes) => invoke("share_torrents", { hashes });
export const trashTorrentFiles = (hash, ids) => invoke("trash_torrent_files", { hash, ids });
export const setLocation = (hashes, location) => invoke("set_location", { hashes, location });
export const uninstallProgram = (name) => invoke("uninstall_program", { name });
export const runSetup = (hash) => invoke("run_setup", { hash });
export const searchTorrents = (query, source = "nyaa") => invoke("search_torrents", { query, source });
export const testNotification = () => invoke("test_notification");
export const getSeeding = () => invoke("get_seeding");
export const setSeeding = (enabled) => invoke("set_seeding", { enabled });
export const openDefaultApps = () => invoke("open_default_apps");
export const stageTorrents = (urls = [], paths = []) => invoke("stage_torrents", { urls, paths });
export const getSources = (hash) => invoke("get_sources", { hash });
export const qbitStatus = () => invoke("qbit_status");
export const setupQbit = () => invoke("setup_qbit");
export const openUrl = (url) => invoke("open_url", { url });
export const setCustomIcon = (path) => invoke("set_custom_icon", { path });
export const getCustomIcon = () => invoke("get_custom_icon");
export const previewTorrent = (url) => invoke("preview_torrent", { url });
export const getSchedule = (titles, kind) => invoke("get_schedule", { titles, kind });
export const organizeShows = (groupKey = null) => invoke("organize_shows", { groupKey });
// Téléchargements directs (liens http) — onglet « Téléchargements »
export const getDownloads = () => invoke("get_downloads");
export const addDownloads = (urls, format, section) => invoke("add_downloads", { urls, format, section });
export const mediaInfo = (url) => invoke("media_info", { url });
export const mediaWave = (id) => invoke("media_wave", { id });
export const compressInfo = (path) => invoke("compress_info", { path });
export const compressVideo = (path, target) => invoke("compress_video", { path, target });
export const clipOpen = (path) => invoke("clip_open", { path });
export const clipProxy = (id) => invoke("clip_proxy", { id });
export const clipWave = (id, audio) => invoke("clip_wave", { id, audio });
export const clipAsset = (path) => invoke("clip_asset", { path });
export const clipExport = (path, segs, height, name, layers = [], music = [], volume = 1) => invoke("clip_export", { path, segs, height, name, layers, music, volume });
export const downloadAction = (ids, action) => invoke("download_action", { ids, action });
export const moveDownloads = (ids, dir) => invoke("move_downloads", { ids, dir });
export const nxbrewLinks = (page) => invoke("nxbrew_links", { page });
// Épisodes vus / en cours (historique de VLC)
export const getWatch = (paths) => invoke("get_watch", { paths });
export const setWatched = (paths, watched) => invoke("set_watched", { paths, watched });
// Lecture suivie dans VLC (position exacte, pistes audio / sous-titres retenues par série)
export const playFiles = withApp("vlc", "NO_VLC", (hash, paths) => invoke("play_files", { hash, paths }));
export const openPath = withApp("reader", "NO_READER", (path) => invoke("open_path", { path }));
export const readBook = withApp("reader", "NO_READER", (path) => invoke("read_book", { path }));
export const appsList = () => invoke("apps_list");
export const appInstall = (id) => invoke("app_install", { id });

// ---------- émulateurs ----------
export const emuList = () => invoke("emu_list");
export const emuDetect = (path) => invoke("emu_detect", { path });
export const emuFindRoms = (path) => invoke("emu_find_roms", { path });
export const emuInstall = (id) => invoke("emu_install", { id });
export const emuPlay = (path, id) => invoke("emu_play", { path, id });
export const emuSetExe = (id, exe) => invoke("emu_set_exe", { id, exe });
export const emuRemove = (id) => invoke("emu_remove", { id });
export const emuTakePending = () => invoke("emu_take_pending");
export const romVersions = (page) => invoke("rom_versions", { page });
/** meta : { title, platform, thumb } du jeu (titre, console et jaquette dans l'onglet DL) */
export const romDownload = (page, meta = {}) => invoke("rom_download", { page, title: meta.title ?? "", platform: meta.platform ?? "", thumb: meta.thumb ?? "" });
export const dlExtract = (id) => invoke("dl_extract", { id });
export const dlSetKind = (id, kind) => invoke("dl_set_kind", { id, kind });
export const trendsCatalog = (kind) => invoke("trends_catalog", { kind });
export const sitePopular = (source, genre) => invoke("site_popular", { source, genre });
export const romLibrary = () => invoke("rom_library");
export const romScan = () => invoke("rom_scan");
export const diskAnalyze = (mount) => invoke("disk_analyze", { mount });
export const diskAnalyzeCancel = () => invoke("disk_analyze_cancel");
export const trashPath = (path) => invoke("trash_path", { path });
export const hostDownload = (url, meta = {}) => invoke("host_download", { url, title: meta.title ?? null, platform: meta.platform ?? null, thumb: meta.thumb ?? null });
/** Lien d'hébergeur (1fichier, DataNodes, ouo.io…) : passe par la fenêtre qui fait les clics */
export const isHostLink = (u) => /^https?:\/\/[^/]*(1fichier\.com|datanodes\.to|ouo\.(io|press)|multiup\.|filekeeper|gofile\.io|buzzheavier|mediafire\.com|pixeldrain)/i.test(u);
export const emptyRecycleBin = (mount) => invoke("empty_recycle_bin", { mount });
