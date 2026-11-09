import { invoke } from "@tauri-apps/api/core";

export const getTorrents = () => invoke("get_torrents");
export const torrentAction = (hashes, action) => invoke("torrent_action", { hashes, action });
export const getFiles = (hash) => invoke("get_files", { hash });
export const setFilePriority = (hash, ids, priority) => invoke("set_file_priority", { hash, ids, priority });
export const getCategories = () => invoke("get_categories");
export const setCategory = (hashes, category) => invoke("set_category", { hashes, category });
export const reveal = (path, fallback) => invoke("reveal", { path, fallback });
export const openPath = (path) => invoke("open_path", { path });
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
export const play = (hashes) => invoke("play", { hashes });
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
export const playFiles = (hash, paths) => invoke("play_files", { hash, paths });

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
export const romDownload = (page) => invoke("rom_download", { page });
