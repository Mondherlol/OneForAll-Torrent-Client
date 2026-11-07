mod artwork;
mod assoc;
mod config;
mod customicon;
mod installed;
mod media;
mod qbit;
mod qsetup;
mod scan;
mod search;

use artwork::{Art, Artwork};
use config::{category_for, Config, Settings};
use media::{detect_kind, Kind, Media};
use qbit::{Category, Qbit, TorrentFile};
use serde::Serialize;
use std::cmp::Ordering as CmpOrdering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager, State};

const KINDS: [&str; 7] = ["film", "series", "anime", "game", "software", "book", "audio"];
const VIDEO_EXT: &[&str] = &["mkv", "mp4", "avi", "m4v", "mov", "wmv", "webm", "ts", "m2ts", "flv", "mpg", "mpeg"];
const AUDIO_EXT: &[&str] = &["mp3", "flac", "m4a", "m4b", "ogg", "opus", "wav", "aac"];
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

struct AppState {
    qbit: Qbit,
    cfg: Mutex<Config>,
    art: Artwork,
    /// Torrents déjà vus par la tâche de fond (None = premier passage pas encore fait)
    seen: Mutex<Option<HashSet<String>>>,
    /// Séries pour lesquelles le mode séquentiel a déjà été réglé
    seq_done: Mutex<HashSet<String>>,
    /// hash → (taille sélectionnée, fichiers cochés, fichiers au total)
    file_counts: Mutex<HashMap<String, (i64, u32, u32)>>,
    /// hash → type déduit du contenu (ex. un dossier rempli de .cbr = livre)
    content_kind: Mutex<HashMap<String, ContentInfo>>,
    /// Dernière progression vue de chaque torrent (pour la notification « terminé »)
    last_progress: Mutex<HashMap<String, (f64, String)>>,
}

impl AppState {
    fn cfg(&self) -> Config {
        self.cfg.lock().unwrap().clone()
    }
}

fn ext_of(name: &str) -> String {
    Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}
fn is_video(name: &str) -> bool {
    VIDEO_EXT.contains(&ext_of(name).as_str())
}
fn is_media(name: &str) -> bool {
    let e = ext_of(name);
    VIDEO_EXT.contains(&e.as_str()) || AUDIO_EXT.contains(&e.as_str())
}

const BOOK_EXT: &[&str] = &["cbz", "cbr", "cb7", "epub", "pdf", "mobi", "azw3"];

/// Formats de jeux pour consoles : extension → console
const CONSOLE_EXT: &[(&str, &str)] = &[
    ("nsp", "Switch"),
    ("nsz", "Switch"),
    ("xci", "Switch"),
    ("xcz", "Switch"),
    ("3ds", "3DS"),
    ("cia", "3DS"),
    ("cci", "3DS"),
    ("nds", "DS"),
    ("gba", "Game Boy"),
    ("gbc", "Game Boy"),
    ("gb", "Game Boy"),
    ("wbfs", "Wii"),
    ("rvz", "Wii"),
    ("wad", "Wii"),
    ("wua", "Wii U"),
    ("cso", "PSP"),
    ("pkg", "PlayStation"),
    ("chd", "Rétro"),
];

#[derive(Clone, Default)]
struct ContentInfo {
    kind: Option<Kind>,
    /// ex. « NSP », « XCI »
    format: Option<String>,
    /// ex. « Switch »
    platform: Option<String>,
}

/// Ce que contient vraiment le torrent : surtout des BD/livres → livre, surtout de l'audio → audio,
/// des fichiers de console (.nsp, .xci…) → jeu console (pas d'installation Windows).
fn content_info(files: &[TorrentFile]) -> ContentInfo {
    let total: i64 = files.iter().map(|f| f.size).sum();
    if total <= 0 {
        return ContentInfo::default();
    }
    let share = |exts: &[&str]| files.iter().filter(|f| exts.contains(&ext_of(&f.name).as_str())).map(|f| f.size).sum::<i64>() as f64 / total as f64;
    // Format console le plus présent (en taille)
    let console = CONSOLE_EXT
        .iter()
        .map(|(e, p)| (*e, *p, files.iter().filter(|f| ext_of(&f.name) == *e).map(|f| f.size).sum::<i64>()))
        .filter(|(_, _, s)| *s as f64 > total as f64 * 0.3)
        .max_by_key(|(_, _, s)| *s);
    if let Some((ext, platform, _)) = console {
        return ContentInfo { kind: Some(Kind::Game), format: Some(ext.to_uppercase()), platform: Some(platform.into()) };
    }
    let (book, audio, video) = (share(BOOK_EXT), share(AUDIO_EXT), share(VIDEO_EXT));
    let kind = if book > 0.5 {
        Some(Kind::Book)
    } else if audio > 0.5 && video < 0.1 {
        Some(Kind::Audio)
    } else {
        None
    };
    ContentInfo { kind, ..Default::default() }
}

/// Tri « naturel » : « E2 » avant « E10 ».
fn nat_cmp(a: &str, b: &str) -> CmpOrdering {
    fn chunks(s: &str) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        for c in s.to_lowercase().chars() {
            let d = c.is_ascii_digit();
            match out.last_mut() {
                Some((was, buf)) if *was == d => buf.push(c),
                _ => out.push((d, c.to_string())),
            }
        }
        out
    }
    let (ca, cb) = (chunks(a), chunks(b));
    for (x, y) in ca.iter().zip(cb.iter()) {
        let o = match (x.0, y.0) {
            (true, true) => x.1.parse::<u64>().unwrap_or(0).cmp(&y.1.parse::<u64>().unwrap_or(0)),
            _ => x.1.cmp(&y.1),
        };
        if o != CmpOrdering::Equal {
            return o;
        }
    }
    ca.len().cmp(&cb.len())
}

fn file_path(save_path: &str, f: &TorrentFile) -> PathBuf {
    Path::new(save_path).join(f.name.replace('/', "\\"))
}

// ---------- torrents ----------

#[tauri::command]
async fn get_torrents(state: State<'_, AppState>) -> Result<Vec<Media>, String> {
    let raw = state.qbit.torrents().await?;
    let mut media: Vec<Media> = raw.iter().cloned().map(Media::from_raw).collect();

    // Torrents partiels : combien de fichiers (épisodes) sont cochés ?
    for (m, r) in media.iter_mut().zip(&raw) {
        if !r.has_metadata || (r.total_size as f64) <= r.size as f64 * 1.001 {
            continue;
        }
        let cached = state.file_counts.lock().unwrap().get(&r.hash).filter(|c| c.0 == r.size).copied();
        let counts = match cached {
            Some(c) => c,
            None => {
                let Ok(files) = state.qbit.files(&r.hash).await else { continue };
                let videos: Vec<&TorrentFile> = files.iter().filter(|f| is_video(&f.name)).collect();
                let set: Vec<&TorrentFile> = if videos.len() >= 2 { videos } else { files.iter().collect() };
                let c = (r.size, set.iter().filter(|f| f.priority > 0).count() as u32, set.len() as u32);
                state.file_counts.lock().unwrap().insert(r.hash.clone(), c);
                c
            }
        };
        m.files_sel = Some(counts.1);
        m.files_total = Some(counts.2);
    }

    // Type déduit du contenu des fichiers (prioritaire sur le nom)
    for (m, r) in media.iter_mut().zip(&raw) {
        if !r.has_metadata {
            continue;
        }
        let cached = state.content_kind.lock().unwrap().get(&r.hash).cloned();
        let info = match cached {
            Some(i) => i,
            None => {
                let Ok(files) = state.qbit.files(&r.hash).await else { continue };
                let i = content_info(&files);
                state.content_kind.lock().unwrap().insert(r.hash.clone(), i.clone());
                i
            }
        };
        if let Some(k) = info.kind.filter(|k| *k != m.kind) {
            m.set_kind(k);
        }
        m.format = info.format.or(m.format.take());
        m.platform = info.platform.or(m.platform.take());
    }

    // Type corrigé par la source des affiches (animation japonaise → animé, trouvé sur Steam → jeu)
    let hints = state.art.kind_hints();
    for m in media.iter_mut() {
        match (m.kind, hints.get(&m.group_key).map(String::as_str)) {
            (Kind::Series, Some("anime")) => m.kind = Kind::Anime,
            (Kind::Other, Some("game")) => m.kind = Kind::Game,
            _ => {}
        }
    }
    // Un même titre ne peut pas être à moitié animé, à moitié série
    let anime: HashSet<String> = media.iter().filter(|m| m.kind == Kind::Anime).map(|m| m.group_key.clone()).collect();
    for m in media.iter_mut().filter(|m| m.kind == Kind::Series && anime.contains(&m.group_key)) {
        m.kind = Kind::Anime;
    }
    // Option : pas de distinction animés / séries
    if !state.cfg().split_anime {
        for m in media.iter_mut().filter(|m| m.kind == Kind::Anime) {
            m.kind = Kind::Series;
        }
    }

    // Jeux et logiciels : déjà installés ?
    let progs = installed::programs();
    for m in media.iter_mut().filter(|m| matches!(m.kind, Kind::Game | Kind::Software)) {
        m.installed = installed::find(&m.title, &progs).cloned();
    }
    Ok(media)
}

#[tauri::command]
async fn torrent_action(state: State<'_, AppState>, hashes: Vec<String>, action: String) -> Result<(), String> {
    state.qbit.action(&hashes, &action).await
}

#[derive(Serialize)]
struct TrackerInfo {
    url: String,
    status: &'static str,
    ok: bool,
    message: String,
    seeds: i64,
    leeches: i64,
}

#[derive(Serialize)]
struct PeerInfo {
    ip: String,
    client: String,
    country: String,
    progress: f64,
    dl: i64,
    up: i64,
    /// pair qui possède déjà tout le fichier
    seed: bool,
}

#[derive(Serialize)]
struct Sources {
    trackers: Vec<TrackerInfo>,
    peers: Vec<PeerInfo>,
}

/// Trackers et pairs d'un torrent (onglet « Sources »).
#[tauri::command]
async fn get_sources(state: State<'_, AppState>, hash: String) -> Result<Sources, String> {
    let tr = state.qbit.trackers(&hash).await?;
    let trackers = tr
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["url"].as_str().is_some_and(|u| !u.starts_with("** [")))
        .map(|t| {
            let (status, ok) = match t["status"].as_i64().unwrap_or(0) {
                0 => ("Désactivé", false),
                1 => ("Pas encore contacté", false),
                2 => ("Fonctionne", true),
                3 => ("Mise à jour…", true),
                _ => ("Ne fonctionne pas", false),
            };
            let num = |k: &str| t[k].as_i64().unwrap_or(-1);
            TrackerInfo {
                url: t["url"].as_str().unwrap_or_default().to_string(),
                status,
                ok,
                message: t["msg"].as_str().unwrap_or_default().to_string(),
                seeds: num("num_seeds"),
                leeches: num("num_leeches"),
            }
        })
        .collect();
    let pv = state.qbit.peers(&hash).await?;
    let mut peers: Vec<PeerInfo> = pv["peers"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
        .map(|p| PeerInfo {
            ip: p["ip"].as_str().unwrap_or_default().to_string(),
            client: p["client"].as_str().unwrap_or_default().to_string(),
            country: p["country"].as_str().unwrap_or_default().to_string(),
            progress: p["progress"].as_f64().unwrap_or(0.0),
            dl: p["dl_speed"].as_i64().unwrap_or(0),
            up: p["up_speed"].as_i64().unwrap_or(0),
            seed: p["progress"].as_f64().unwrap_or(0.0) >= 1.0,
        })
        .collect();
    peers.sort_by(|a, b| (b.dl + b.up).cmp(&(a.dl + a.up)).then(b.progress.total_cmp(&a.progress)));
    Ok(Sources { trackers, peers })
}

#[tauri::command]
async fn get_files(state: State<'_, AppState>, hash: String) -> Result<Vec<TorrentFile>, String> {
    state.qbit.files(&hash).await
}

#[tauri::command]
async fn set_file_priority(state: State<'_, AppState>, hash: String, ids: Vec<i64>, priority: i64) -> Result<(), String> {
    state.qbit.set_file_priority(&hash, &ids, priority).await
}

/// Catégories qBittorrent + celles de OneForAll (créées au premier usage).
#[tauri::command]
async fn get_categories(state: State<'_, AppState>) -> Result<Vec<Category>, String> {
    let mut cats = state.qbit.categories().await?;
    for k in KINDS {
        if let Some(n) = category_for(k) {
            cats.entry(n.to_string()).or_insert_with(|| Category { name: n.to_string(), save_path: String::new() });
        }
    }
    let mut v: Vec<Category> = cats.into_values().collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(v)
}

#[tauri::command]
async fn set_category(state: State<'_, AppState>, hashes: Vec<String>, category: String) -> Result<(), String> {
    if !category.is_empty() && !state.qbit.categories().await?.contains_key(&category) {
        // Catégorie OneForAll pas encore créée : on la crée avec le dossier réglé pour ce type
        let cfg = state.cfg();
        let folder = KINDS
            .iter()
            .find(|k| category_for(k) == Some(category.as_str()))
            .and_then(|k| cfg.folders.get(*k).cloned())
            .unwrap_or_default();
        state.qbit.upsert_category(&category, &folder).await?;
    }
    state.qbit.set_category(&hashes, &category).await?;
    if !category.is_empty() {
        // En mode automatique, qBittorrent déplace les fichiers dans le dossier de la catégorie.
        state.qbit.set_auto_tmm(&hashes, true).await?;
    }
    Ok(())
}

// ---------- ajout de torrents ----------

#[tauri::command]
async fn search_torrents(state: State<'_, AppState>, query: String, source: String) -> Result<Vec<search::SearchResult>, String> {
    let client = reqwest::Client::builder()
        .user_agent("OneForAll/0.4")
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    match source.as_str() {
        "c411" => search::torznab(&client, "https://c411.org", &state.cfg().c411_key, query.trim(), "c411").await,
        _ => search::nyaa(&client, query.trim()).await,
    }
}

#[derive(Serialize)]
struct Preview {
    name: String,
    hash: Option<String>,
    files: Vec<scan::TorrentEntry>,
    total: i64,
}

/// Aperçu d'un résultat de recherche : liste des fichiers du .torrent, sans l'ajouter.
#[tauri::command]
async fn preview_torrent(url: String) -> Result<Preview, String> {
    if !url.starts_with("http") {
        return Err("Aperçu impossible pour un lien magnet (les fichiers ne sont connus qu'après ajout).".into());
    }
    let client = reqwest::Client::builder().user_agent("OneForAll/0.6").timeout(Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
    let res = client.get(&url).send().await.map_err(|e| format!("Téléchargement impossible : {e}"))?;
    if !res.status().is_success() {
        return Err(format!("Le site a répondu {}", res.status()));
    }
    let bytes = res.bytes().await.map_err(|e| e.to_string())?;
    let (name, mut files) = scan::torrent_files(&bytes).ok_or("Ce fichier n'est pas un .torrent valide")?;
    files.sort_by(|a, b| nat_cmp(&a.path, &b.path));
    let total = files.iter().map(|f| f.size).sum();
    Ok(Preview { name, hash: scan::infohash(&bytes), files, total })
}

#[derive(Serialize, Clone)]
struct Staged {
    /// Torrents ajoutés SANS démarrer : l'interface propose de choisir les fichiers, puis les lance
    hashes: Vec<String>,
    errors: Vec<String>,
}

/// Ajoute des torrents (liens magnet, liens .torrent, fichiers .torrent) sans les démarrer.
async fn stage(st: &AppState, urls: &[String], paths: &[String]) -> Staged {
    let mut out = Staged { hashes: Vec::new(), errors: Vec::new() };
    let add_bytes = |name: String, bytes: Vec<u8>| async move { (scan::infohash(&bytes), st.qbit.add_file(&name, bytes, true).await) };
    for u in urls.iter().map(|u| u.trim()).filter(|u| !u.is_empty()) {
        if u.to_lowercase().starts_with("magnet:") {
            match st.qbit.add_magnet_staged(u).await {
                Ok(()) => out.hashes.extend(scan::magnet_hash(u)),
                Err(e) => out.errors.push(e),
            }
        } else if u.starts_with("http") {
            // Lien d'un fichier .torrent (ex. nyaa.si) : on le télécharge pour connaître son contenu
            let client = reqwest::Client::builder().user_agent("OneForAll/0.5").timeout(Duration::from_secs(20)).build();
            let res = match client {
                Ok(c) => c.get(u).send().await.map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            match res {
                Ok(r) if r.status().is_success() => match r.bytes().await {
                    Ok(b) => match add_bytes("lien.torrent".into(), b.to_vec()).await {
                        (h, Ok(())) => out.hashes.extend(h),
                        (_, Err(e)) => out.errors.push(e),
                    },
                    Err(e) => out.errors.push(e.to_string()),
                },
                Ok(r) => out.errors.push(format!("Téléchargement du .torrent impossible ({})", r.status())),
                Err(e) => out.errors.push(e),
            }
        } else {
            out.errors.push(format!("Lien non reconnu : {u}"));
        }
    }
    for p in paths {
        let path = Path::new(p);
        if !p.to_lowercase().ends_with(".torrent") || !path.is_file() {
            continue;
        }
        let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "fichier.torrent".into());
        match std::fs::read(path) {
            Ok(bytes) => match add_bytes(name, bytes).await {
                (h, Ok(())) => out.hashes.extend(h),
                (_, Err(e)) => out.errors.push(e),
            },
            Err(e) => out.errors.push(e.to_string()),
        }
    }
    out
}

#[tauri::command]
async fn stage_torrents(state: State<'_, AppState>, urls: Vec<String>, paths: Vec<String>) -> Result<Staged, String> {
    Ok(stage(&state, &urls, &paths).await)
}

/// Déplace les fichiers des torrents dans un autre dossier.
#[tauri::command]
async fn set_location(state: State<'_, AppState>, hashes: Vec<String>, location: String) -> Result<(), String> {
    state.qbit.set_location(&hashes, &location).await
}

#[tauri::command]
fn uninstall_program(name: String) -> Result<(), String> {
    installed::uninstall(&name)
}

/// Lance l'installateur contenu dans le torrent (setup.exe, install.exe…).
#[tauri::command]
async fn run_setup(state: State<'_, AppState>, hash: String) -> Result<(), String> {
    let ts = state.qbit.torrents().await?;
    let save = ts.iter().find(|t| t.hash == hash).map(|t| t.save_path.clone()).ok_or("Torrent introuvable")?;
    let files = state.qbit.files(&hash).await?;
    let mut setups: Vec<&TorrentFile> = files
        .iter()
        .filter(|f| {
            let n = Path::new(&f.name).file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
            n.ends_with(".exe") && (n.starts_with("setup") || n.starts_with("install"))
        })
        .collect();
    // Le moins profond d'abord (le setup principal, pas celui d'un redistribuable)
    setups.sort_by_key(|f| f.name.matches('/').count());
    let f = setups.first().ok_or("Aucun setup.exe trouvé dans ce torrent")?;
    let p = file_path(&save, f);
    if !p.is_file() {
        return Err("Le setup n'est pas (encore) sur le disque".into());
    }
    installed::invalidate();
    explorer(format!("\"{}\"", p.display()))
}

/// Supprime du disque (vers la Corbeille) des fichiers décochés d'un torrent.
#[tauri::command]
async fn trash_torrent_files(state: State<'_, AppState>, hash: String, ids: Vec<i64>) -> Result<(usize, i64), String> {
    let ts = state.qbit.torrents().await?;
    let save = ts.iter().find(|t| t.hash == hash).map(|t| t.save_path.clone()).ok_or("Torrent introuvable")?;
    let files = state.qbit.files(&hash).await?;
    let wanted: HashSet<i64> = ids.into_iter().collect();
    // Par sécurité, seuls les fichiers décochés (priorité 0) peuvent être supprimés.
    let targets: Vec<(PathBuf, i64)> = files
        .iter()
        .filter(|f| wanted.contains(&f.index) && f.priority == 0)
        .map(|f| (file_path(&save, f), f.size))
        .filter(|(p, _)| p.is_file())
        .collect();
    if targets.is_empty() {
        return Ok((0, 0));
    }
    let paths: Vec<&PathBuf> = targets.iter().map(|(p, _)| p).collect();
    trash::delete_all(&paths).map_err(|e| format!("Suppression impossible : {e}"))?;
    Ok((targets.len(), targets.iter().map(|(_, s)| s).sum()))
}

// ---------- lecture, explorateur, partage ----------

fn explorer(arg: String) -> Result<(), String> {
    let mut cmd = std::process::Command::new("explorer");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.raw_arg(arg);
    }
    #[cfg(not(windows))]
    cmd.arg(arg);
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Ouvre le dossier du torrent (ou sélectionne le fichier s'il s'agit d'un fichier seul).
#[tauri::command]
fn reveal(path: String, fallback: String) -> Result<(), String> {
    let p = Path::new(&path);
    if p.is_dir() {
        explorer(format!("\"{path}\""))
    } else if p.exists() {
        explorer(format!("/select,\"{path}\""))
    } else if Path::new(&fallback).is_dir() {
        explorer(format!("\"{fallback}\""))
    } else {
        Err("Le dossier n'existe plus.".into())
    }
}

/// Ouvre un fichier avec l'application par défaut (ou un dossier).
#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    if Path::new(&path).exists() {
        explorer(format!("\"{path}\""))
    } else {
        Err("Ce fichier n'existe pas (encore).".into())
    }
}

fn find_vlc() -> Option<PathBuf> {
    let mut c = vec![];
    for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Ok(base) = std::env::var(var) {
            c.push(Path::new(&base).join("VideoLAN").join("VLC").join("vlc.exe"));
            c.push(Path::new(&base).join("Programs").join("VideoLAN").join("VLC").join("vlc.exe"));
        }
    }
    c.into_iter().find(|p| p.is_file())
}

/// Lit les vidéos/musiques cochées et téléchargées, dans l'ordre, dans VLC (en playlist)
/// ou avec le lecteur par défaut. Renvoie le nombre de fichiers.
#[tauri::command]
async fn play(state: State<'_, AppState>, hashes: Vec<String>) -> Result<usize, String> {
    let ts = state.qbit.torrents().await?;
    let mut complete = Vec::new();
    let mut partial = Vec::new();
    for h in &hashes {
        let Some(t) = ts.iter().find(|t| &t.hash == h) else { continue };
        let mut files = state.qbit.files(h).await?;
        files.retain(|f| f.priority > 0 && f.progress > 0.0 && is_media(&f.name));
        files.sort_by(|a, b| nat_cmp(&a.name, &b.name));
        for f in files {
            let p = file_path(&t.save_path, &f);
            if p.is_file() {
                if f.progress >= 0.999 { complete.push(p) } else { partial.push(p) }
            }
        }
    }
    // On lit ce qui est complet ; à défaut, le début de ce qui est en cours.
    let list = if complete.is_empty() { partial } else { complete };
    if list.is_empty() {
        return Err("Aucune vidéo ou musique téléchargée à lire.".into());
    }
    match find_vlc() {
        Some(vlc) => {
            std::process::Command::new(vlc).args(&list).spawn().map_err(|e| e.to_string())?;
        }
        None => explorer(format!("\"{}\"", list[0].display()))?,
    }
    Ok(list.len())
}

fn safe_name(s: &str) -> String {
    let n: String = s.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    n.trim().chars().take(150).collect()
}

/// Exporte les .torrent et les place dans le presse-papiers comme fichiers (à coller dans WhatsApp, Discord…).
#[tauri::command]
async fn share_torrents(state: State<'_, AppState>, hashes: Vec<String>) -> Result<usize, String> {
    let ts = state.qbit.torrents().await?;
    let dir = std::env::temp_dir().join("OneForAll-partage");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut paths = Vec::new();
    for h in &hashes {
        let name = ts.iter().find(|t| &t.hash == h).map(|t| t.name.clone()).unwrap_or_else(|| h.clone());
        let bytes = state.qbit.export(h).await?;
        let p = dir.join(format!("{}.torrent", safe_name(&name)));
        std::fs::write(&p, bytes).map_err(|e| e.to_string())?;
        paths.push(p);
    }
    let list = paths.iter().map(|p| format!("'{}'", p.display().to_string().replace('\'', "''"))).collect::<Vec<_>>().join(",");
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &format!("Set-Clipboard -LiteralPath {list}")]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("Presse-papiers : {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(paths.len())
}

// ---------- réglages ----------

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    let cfg = state.cfg();
    let cats = state.qbit.categories().await.unwrap_or_default();
    let mut folders = cfg.folders.clone();
    for k in KINDS {
        let empty = folders.get(k).is_none_or(|f| f.is_empty());
        if let (true, Some(c)) = (empty, category_for(k).and_then(|n| cats.get(n))) {
            folders.insert(k.into(), c.save_path.replace('/', "\\"));
        }
    }
    Ok(Settings {
        folders,
        auto_sort: cfg.auto_sort,
        seq_episodes: cfg.seq_episodes,
        tmdb_key: cfg.tmdb_key,
        manage_qbit: cfg.manage_qbit,
        split_anime: cfg.split_anime,
        c411_key: cfg.c411_key,
    })
}

#[tauri::command]
async fn save_settings(state: State<'_, AppState>, settings: Settings) -> Result<(), String> {
    let cfg = {
        let mut c = state.cfg.lock().unwrap();
        c.folders = settings.folders.into_iter().map(|(k, v)| (k, v.trim().to_string())).collect();
        c.auto_sort = settings.auto_sort;
        c.seq_episodes = settings.seq_episodes;
        c.tmdb_key = settings.tmdb_key.trim().to_string();
        c.manage_qbit = settings.manage_qbit;
        c.split_anime = settings.split_anime;
        c.c411_key = settings.c411_key.trim().to_string();
        c.clone()
    };
    config::save(&cfg)?;
    state.seq_done.lock().unwrap().clear();
    for (kind, folder) in &cfg.folders {
        if let (Some(cat), false) = (category_for(kind), folder.is_empty()) {
            state.qbit.upsert_category(cat, folder).await?;
        }
    }
    Ok(())
}

#[tauri::command]
async fn get_limits(state: State<'_, AppState>) -> Result<(i64, i64), String> {
    state.qbit.limits().await
}

#[tauri::command]
async fn set_limits(state: State<'_, AppState>, dl: i64, up: i64) -> Result<(), String> {
    state.qbit.set_limits(dl, up).await
}

#[tauri::command]
async fn get_seeding(state: State<'_, AppState>) -> Result<bool, String> {
    state.qbit.seeding_enabled().await
}

/// Coupe (ou relance) le partage : arrête tout de suite les torrents terminés, et les suivants
/// s'arrêteront dès la fin de leur téléchargement.
#[tauri::command]
async fn set_seeding(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    state.qbit.set_seeding(enabled).await?;
    let ts = state.qbit.torrents().await?;
    let done: Vec<String> = ts
        .iter()
        .filter(|t| t.progress >= 1.0 && t.state != "missingFiles" && t.state != "error")
        .filter(|t| if enabled { t.state.starts_with("stopped") || t.state.starts_with("paused") } else { !t.state.starts_with("stopped") && !t.state.starts_with("paused") })
        .map(|t| t.hash.clone())
        .collect();
    if !done.is_empty() {
        state.qbit.action(&done, if enabled { "resume" } else { "pause" }).await?;
    }
    Ok(())
}

#[tauri::command]
fn get_disks() -> Vec<scan::Disk> {
    scan::disks()
}

// ---------- fichiers .torrent ----------

#[derive(Serialize)]
struct ScanItem {
    #[serde(flatten)]
    info: scan::TorrentFileInfo,
    in_use: bool,
}

#[tauri::command]
async fn scan_torrents(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<Vec<ScanItem>, String> {
    // On récupère d'abord la liste actuelle : sans elle, impossible de savoir ce qui est utilisé.
    let current = state.qbit.torrents().await?;
    let mut known = HashSet::new();
    for t in &current {
        for h in [&t.hash, &t.infohash_v1, &t.infohash_v2] {
            if !h.is_empty() {
                known.insert(h.to_lowercase());
            }
        }
    }

    let counter = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicBool::new(false));
    {
        let (c, d, app) = (counter.clone(), done.clone(), app.clone());
        tauri::async_runtime::spawn(async move {
            while !d.load(Ordering::Relaxed) {
                let _ = app.emit("scan-progress", c.load(Ordering::Relaxed));
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        });
    }
    let c = counter.clone();
    let found = tauri::async_runtime::spawn_blocking(move || scan::find_torrents(&c)).await.map_err(|e| e.to_string());
    done.store(true, Ordering::Relaxed);
    let found = found?;

    let mut out: Vec<ScanItem> = found
        .into_iter()
        .map(|info| {
            let v1 = info.v1.as_ref().is_some_and(|h| known.contains(h));
            let v2 = info.v2.as_ref().is_some_and(|h| known.contains(h) || known.contains(&h[..40]));
            ScanItem { in_use: v1 || v2, info }
        })
        .collect();
    out.sort_by(|a, b| a.in_use.cmp(&b.in_use).then(b.info.modified.cmp(&a.info.modified)));
    Ok(out)
}

/// Envoie les fichiers .torrent à la Corbeille (et uniquement des .torrent).
#[tauri::command]
fn trash_files(paths: Vec<String>) -> Result<usize, String> {
    let ok: Vec<&String> = paths.iter().filter(|p| p.to_lowercase().ends_with(".torrent") && Path::new(p).is_file()).collect();
    if ok.is_empty() {
        return Ok(0);
    }
    trash::delete_all(&ok).map_err(|e| e.to_string())?;
    Ok(ok.len())
}

// ---------- images ----------

#[tauri::command]
async fn get_artwork(state: State<'_, AppState>, key: String, kind: String, title: String, year: Option<u32>) -> Result<Option<Art>, String> {
    // Clé personnelle si l'utilisateur en a mis une, sinon celle intégrée à la compilation
    let mut tmdb = state.cfg().tmdb_key;
    if tmdb.is_empty() {
        tmdb = option_env!("ONEFORALL_TMDB_KEY").unwrap_or_default().to_string();
    }
    Ok(state.art.get(&key, &kind, &title, year, &tmdb).await)
}

#[tauri::command]
fn forget_artwork(state: State<'_, AppState>, key: String) {
    state.art.forget(&key);
}

// ---------- qBittorrent : lancement ----------

fn qbit_exe(cfg: &Config) -> Option<PathBuf> {
    if !cfg.qbit_exe.is_empty() {
        return Some(PathBuf::from(&cfg.qbit_exe)).filter(|p| p.is_file());
    }
    ["ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .map(|b| Path::new(&b).join("qBittorrent").join("qbittorrent.exe"))
        .find(|p| p.is_file())
}

/// Modifie une valeur du fichier .ini de qBittorrent (uniquement quand il est fermé).
fn set_ini(path: &Path, section: &str, key: &str, value: &str) -> std::io::Result<()> {
    let text = std::fs::read_to_string(path)?;
    let text = text.trim_start_matches('\u{feff}');
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let header = format!("[{section}]");
    let entry = format!("{key}={value}");
    match lines.iter().position(|l| l.trim() == header) {
        Some(start) => {
            let end = lines[start + 1..].iter().position(|l| l.starts_with('[')).map(|i| start + 1 + i).unwrap_or(lines.len());
            match lines[start + 1..end].iter().position(|l| l.starts_with(&format!("{key}="))) {
                Some(i) => lines[start + 1 + i] = entry,
                None => lines.insert(start + 1, entry),
            }
        }
        None => {
            lines.push(String::new());
            lines.push(header);
            lines.push(entry);
        }
    }
    std::fs::write(path, lines.join(nl) + nl)
}

/// Avant de lancer qBittorrent : démarrage réduit dans la zone de notification, et pas de
/// notifications de sa part (c'est OneForAll qui prévient).
fn prepare_qbit_settings() {
    let Ok(appdata) = std::env::var("APPDATA") else { return };
    let ini = Path::new(&appdata).join("qBittorrent").join("qBittorrent.ini");
    if !ini.is_file() {
        return;
    }
    let _ = set_ini(&ini, "Preferences", r"General\StartMinimized", "true");
    let _ = set_ini(&ini, "Preferences", r"General\MinimizeToTray", "true");
    let _ = set_ini(&ini, "Preferences", r"General\CloseToTray", "true");
    let _ = set_ini(&ini, "Preferences", r"General\NoSplashScreen", "true");
    let _ = set_ini(&ini, "GUI", r"Notifications\Enabled", "false");
    // Pas de fenêtre « mise à jour disponible » qui surgit au démarrage
    let _ = set_ini(&ini, "Preferences", r"Advanced\updateCheck", "false");
}

/// PID du qBittorrent lancé par OneForAll (0 = pas encore connu)
#[cfg(windows)]
static QBIT_PID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Windows nous prévient à l'instant où une fenêtre s'affiche : si c'est une fenêtre de
/// qBittorrent, on la cache aussitôt (elle n'a pas le temps d'apparaître à l'écran).
#[cfg(windows)]
fn start_qbit_window_guard() {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetAncestor, GetWindowTextW, GetWindowThreadProcessId, PeekMessageW, ShowWindow, TranslateMessage, MSG, SW_HIDE,
    };

    unsafe extern "system" fn on_show(_h: HWINEVENTHOOK, _ev: u32, hwnd: HWND, id_object: i32, id_child: i32, _t: u32, _ms: u32) {
        if id_object != 0 || id_child != 0 || hwnd.is_null() {
            return; // pas une fenêtre entière
        }
        unsafe {
            if GetAncestor(hwnd, 2 /* GA_ROOT */) != hwnd {
                return; // pas une fenêtre principale
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            let ours = QBIT_PID.load(std::sync::atomic::Ordering::Relaxed);
            let is_qbit = if ours != 0 {
                pid == ours
            } else {
                let mut buf = [0u16; 128];
                let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
                String::from_utf16_lossy(&buf[..n.max(0) as usize]).contains("qBittorrent")
            };
            if is_qbit {
                ShowWindow(hwnd, SW_HIDE);
            }
        }
    }

    std::thread::spawn(|| unsafe {
        // EVENT_OBJECT_SHOW = 0x8002, WINEVENT_OUTOFCONTEXT = 0
        let hook = SetWinEventHook(0x8002, 0x8002, std::ptr::null_mut(), Some(on_show), 0, 0, 0);
        if hook.is_null() {
            return;
        }
        // Le crochet a besoin d'une boucle de messages ; on surveille les 25 premières secondes
        let end = std::time::Instant::now() + Duration::from_secs(25);
        let mut msg: MSG = std::mem::zeroed();
        while std::time::Instant::now() < end {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, 1 /* PM_REMOVE */) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        UnhookWinEvent(hook);
    });
}

#[cfg(not(windows))]
fn start_qbit_window_guard() {}

/// Lance qBittorrent en arrière-plan : sans écran de démarrage, fenêtre cachée dès sa création.
fn launch_qbit(exe: &Path) {
    prepare_qbit_settings();
    start_qbit_window_guard();
    std::thread::sleep(Duration::from_millis(50)); // le temps que le crochet soit en place
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--no-splash");
    if let Ok(child) = cmd.spawn() {
        #[cfg(windows)]
        QBIT_PID.store(child.id(), std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(windows))]
        let _ = child;
    }
}

// ---------- tâche de fond ----------

/// Notification Windows « téléchargement terminé », avec l'affiche du média en vignette.
fn notify_finished(app: &tauri::AppHandle, m: &Media, poster: Option<&Path>) {
    use tauri_winrt_notification::{IconCrop, Sound, Toast};

    let what = match (m.season, m.episode) {
        (Some(s), Some(e)) => format!("Saison {s} · Épisode {e}"),
        (None, Some(e)) => format!("Épisode {e}"),
        _ => match m.kind {
            Kind::Film => "Film".into(),
            Kind::Game => "Jeu".into(),
            Kind::Software => "Logiciel".into(),
            Kind::Book => "Livre".into(),
            Kind::Audio => "Audio".into(),
            _ => String::new(),
        },
    };
    let gb = m.size as f64 / 1_073_741_824.0;
    let size = if gb >= 1.0 { format!("{gb:.1} Go") } else { format!("{:.0} Mo", gb * 1024.0) }.replace('.', ",");
    let line2 = [what.as_str(), size.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · ");

    // Identifiant de l'app installée (raccourci du menu Démarrer créé par l'installateur)
    let app_id = app.config().identifier.clone();
    let mut toast = Toast::new(&app_id)
        .title(&m.title)
        .text1(&line2)
        .text2("Téléchargement terminé ✓")
        .sound(Some(Sound::Default));
    if let Some(p) = poster {
        toast = toast.icon(p, IconCrop::Square, &m.title);
    }
    let _ = toast.show();
}

/// Affiche une notification d'exemple avec le dernier torrent terminé.
#[tauri::command]
async fn test_notification(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let ts = state.qbit.torrents().await?;
    let t = ts.iter().filter(|t| t.progress >= 1.0).max_by_key(|t| t.completion_on).ok_or("Aucun torrent terminé")?;
    let m = Media::from_raw(t.clone());
    let poster = state.art.poster_file(&m.group_key).await;
    notify_finished(&app, &m, poster.as_deref());
    Ok(())
}

async fn background_tick(st: &AppState, app: &tauri::AppHandle) {
    let Ok(ts) = st.qbit.torrents().await else { return };

    // 0. Téléchargements qui viennent de se terminer → notification Windows
    let finished: Vec<Media> = {
        let mut last = st.last_progress.lock().unwrap();
        let mut out = Vec::new();
        for t in &ts {
            let before = last.insert(t.hash.clone(), (t.progress, t.state.clone()));
            // Seulement si on l'a VU en train de télécharger juste avant. Au démarrage, qBittorrent
            // annonce d'abord ses torrents à 0 % (« vérification ») puis à 100 % : ce n'est pas une fin.
            let was_downloading = before.is_some_and(|(p, s)| {
                p < 0.999 && matches!(s.as_str(), "downloading" | "stalledDL" | "forcedDL" | "queuedDL")
            });
            if was_downloading && t.progress >= 0.999 {
                out.push(Media::from_raw(t.clone()));
            }
        }
        out
    };
    for m in finished {
        let poster = st.art.poster_file(&m.group_key).await;
        notify_finished(app, &m, poster.as_deref());
    }
    let cfg = st.cfg();

    // 1. Nouveaux torrents → rangement automatique
    let fresh: Vec<_> = {
        let mut seen = st.seen.lock().unwrap();
        match seen.as_mut() {
            None => {
                *seen = Some(ts.iter().map(|t| t.hash.clone()).collect());
                Vec::new()
            }
            Some(s) => ts.iter().filter(|t| t.has_metadata && s.insert(t.hash.clone())).cloned().collect(),
        }
    };
    if cfg.auto_sort && fresh.iter().any(|t| t.category.is_empty()) {
        let mut cats = st.qbit.categories().await.unwrap_or_default();
        for t in fresh.iter().filter(|t| t.category.is_empty()) {
            let mut kind = detect_kind(&t.name, &t.category).as_str();
            // Pas de dossier « Animes » configuré : les animés vont avec les séries.
            let anime_folder = cfg.folders.get("anime").is_some_and(|f| !f.is_empty());
            if kind == "anime" && (!cfg.split_anime || (!anime_folder && !cats.contains_key("Animes"))) {
                kind = "series";
            }
            let Some(cat) = category_for(kind) else { continue };
            if let Some(folder) = cfg.folders.get(kind).filter(|f| !f.is_empty()) {
                if st.qbit.upsert_category(cat, folder).await.is_ok() {
                    cats.entry(cat.to_string()).or_default();
                }
            }
            if cats.contains_key(cat) {
                let h = vec![t.hash.clone()];
                let _ = st.qbit.set_category(&h, cat).await;
                let _ = st.qbit.set_auto_tmm(&h, true).await;
            }
        }
    }

    // 2. Vidéos et audio en cours → TOUJOURS téléchargés dans l'ordre (épisode 1, puis 2…)
    {
        let (mut seq, mut flpp) = (Vec::new(), Vec::new());
        {
            let mut done = st.seq_done.lock().unwrap();
            for t in &ts {
                if t.progress >= 1.0 || !t.has_metadata || done.contains(&t.hash) {
                    continue;
                }
                if !matches!(detect_kind(&t.name, &t.category), Kind::Series | Kind::Anime | Kind::Film | Kind::Audio) {
                    continue;
                }
                done.insert(t.hash.clone());
                if !t.seq_dl {
                    seq.push(t.hash.clone());
                }
                if !t.f_l_piece_prio {
                    flpp.push(t.hash.clone());
                }
            }
        }
        if !seq.is_empty() {
            let _ = st.qbit.action(&seq, "toggle_seq").await;
        }
        if !flpp.is_empty() {
            let _ = st.qbit.action(&flpp, "toggle_flpp").await;
        }
    }
}

#[derive(Serialize)]
struct QbitStatus {
    installed: bool,
    running: bool,
    reachable: bool,
    configured: bool,
}

/// Où en est qBittorrent ? (pour l'écran « relier qBittorrent »)
#[tauri::command]
async fn qbit_status(state: State<'_, AppState>) -> Result<QbitStatus, String> {
    let cfg = state.cfg();
    Ok(QbitStatus {
        installed: qbit_exe(&cfg).is_some(),
        running: qsetup::is_running(),
        reachable: state.qbit.torrents().await.is_ok(),
        configured: !cfg.password.is_empty(),
    })
}

/// Configure qBittorrent pour OneForAll (Web UI locale + identifiants), puis le (re)lance.
#[tauri::command]
async fn setup_qbit(state: State<'_, AppState>) -> Result<(), String> {
    let cfg = state.cfg();
    let exe = qbit_exe(&cfg).ok_or("qBittorrent n'est pas installé")?;
    let cfg = tauri::async_runtime::spawn_blocking(move || {
        if qsetup::is_running() {
            qsetup::close();
        }
        qsetup::configure(cfg)
    })
    .await
    .map_err(|e| e.to_string())??;
    config::save(&cfg)?;
    state.qbit.set_creds(&cfg);
    *state.cfg.lock().unwrap() = cfg;
    launch_qbit(&exe);
    for _ in 0..60 {
        if state.qbit.torrents().await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err("qBittorrent a été configuré mais ne répond pas encore. Réessaie dans un instant.".into())
}

/// Icône personnalisée (sur ce PC uniquement). path absent = revenir à l'icône d'origine.
#[tauri::command]
fn set_custom_icon(app: tauri::AppHandle, path: Option<String>) -> Result<Option<String>, String> {
    customicon::set(&app, path)
}

#[tauri::command]
fn get_custom_icon() -> Option<String> {
    customicon::current()
}

/// Ouvre une page web (uniquement https) dans le navigateur.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("Adresse refusée".into());
    }
    explorer(format!("\"{url}\""))
}

/// Ouvre la page Windows « Applications par défaut » de OneForAll (pour les liens magnet).
#[tauri::command]
fn open_default_apps() -> Result<(), String> {
    explorer("ms-settings:defaultapps?registeredAppUser=OneForAll".into())
}

/// Ajoute les .torrent / liens magnet reçus au lancement (double-clic sur un fichier, clic sur un lien).
/// Si qBittorrent démarre encore, on attend qu'il soit prêt (40 s max).
fn open_from_args(app: tauri::AppHandle, args: Vec<String>) {
    let (magnets, files) = assoc::torrent_args(&args);
    if magnets.is_empty() && files.is_empty() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let st = app.state::<AppState>();
        for _ in 0..80 {
            if st.qbit.torrents().await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let staged = stage(&st, &magnets, &files).await;
        show_main(&app);
        let _ = app.emit("torrents-staged", staged);
    });
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Icône dans la zone de notification : clic = ouvrir, clic droit = menu (Ouvrir / Quitter).
fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let open = MenuItem::with_id(app, "open", "Ouvrir OneForAll", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quitter (ferme aussi qBittorrent)", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &sep, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("OneForAll")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id.as_ref() {
            "open" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // En cas de plantage, la cause est écrite dans %APPDATA%\OneForAll\crash.log
    std::panic::set_hook(Box::new(|info| {
        let _ = std::fs::create_dir_all(config::data_dir());
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let line = format!("[{secs}] {info}\n{}\n\n", std::backtrace::Backtrace::force_capture());
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(config::data_dir().join("crash.log")) {
            let _ = f.write_all(line.as_bytes());
        }
    }));

    config::migrate_old_data();
    let cfg = config::load();
    let state = AppState {
        qbit: Qbit::new(&cfg),
        cfg: Mutex::new(cfg),
        art: Artwork::new(),
        seen: Mutex::new(None),
        seq_done: Mutex::new(HashSet::new()),
        file_counts: Mutex::new(HashMap::new()),
        content_kind: Mutex::new(HashMap::new()),
        last_progress: Mutex::new(HashMap::new()),
    };
    let app = tauri::Builder::default()
        // Relancer OneForAll alors qu'il tourne déjà → on rouvre simplement la fenêtre
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            show_main(app);
            open_from_args(app.clone(), args);
        }))
        .plugin(tauri_plugin_dialog::init())
        // Mises à jour automatiques depuis les releases GitHub (signées)
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(state)
        // La croix ne quitte pas : la fenêtre est cachée et OneForAll reste dans la zone de notification
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            build_tray(app)?;
            customicon::apply_saved(app.handle());
            // .torrent / magnet: → OneForAll
            std::thread::spawn(assoc::register);
            // Lancé en double-cliquant un .torrent (ou un lien magnet) ?
            open_from_args(app.handle().clone(), std::env::args().skip(1).collect());
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // qBittorrent pas lancé ? On le démarre.
                {
                    let st = handle.state::<AppState>();
                    let cfg = st.cfg();
                    // Premier lancement : qBittorrent fermé et pas encore relié → on le configure en silence
                    if cfg.manage_qbit && cfg.password.is_empty() && qbit_exe(&cfg).is_some() && !qsetup::is_running() {
                        if let Ok(new_cfg) = qsetup::configure(cfg.clone()) {
                            let _ = config::save(&new_cfg);
                            st.qbit.set_creds(&new_cfg);
                            *st.cfg.lock().unwrap() = new_cfg;
                        }
                    }
                    let cfg = st.cfg();
                    if cfg.manage_qbit && !st.qbit.alive().await {
                        if let Some(exe) = qbit_exe(&cfg) {
                            launch_qbit(&exe);
                        }
                    }
                }
                loop {
                    background_tick(&handle.state::<AppState>(), &handle).await;
                    tokio::time::sleep(Duration::from_secs(4)).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_torrents,
            torrent_action,
            get_files,
            get_sources,
            set_file_priority,
            get_categories,
            set_category,
            trash_torrent_files,
            set_location,
            search_torrents,
            stage_torrents,
            preview_torrent,
            uninstall_program,
            run_setup,
            reveal,
            open_path,
            play,
            share_torrents,
            get_settings,
            save_settings,
            get_limits,
            set_limits,
            get_disks,
            get_seeding,
            set_seeding,
            scan_torrents,
            trash_files,
            get_artwork,
            forget_artwork,
            test_notification,
            open_default_apps,
            qbit_status,
            setup_qbit,
            open_url,
            set_custom_icon,
            get_custom_icon,
        ])
        .build(tauri::generate_context!())
        .expect("erreur au lancement de OneForAll");

    app.run(|handle, event| {
        // Fermeture de OneForAll → fermeture propre de qBittorrent
        if let tauri::RunEvent::Exit = event {
            let st = handle.state::<AppState>();
            if st.cfg().manage_qbit {
                tauri::async_runtime::block_on(async {
                    let _ = tokio::time::timeout(Duration::from_secs(4), st.qbit.shutdown()).await;
                });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::nat_cmp;
    use std::cmp::Ordering;

    #[test]
    fn natural_sort() {
        assert_eq!(nat_cmp("Ep 2.mkv", "Ep 10.mkv"), Ordering::Less);
        assert_eq!(nat_cmp("S01E09", "S01E10"), Ordering::Less);
        assert_eq!(nat_cmp("b", "a"), Ordering::Greater);
    }
}