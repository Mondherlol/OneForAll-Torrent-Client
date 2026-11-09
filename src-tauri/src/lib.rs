mod artwork;
mod assoc;
mod chapters;
mod clip;
mod config;
mod customicon;
mod direct;
mod emu;
mod installed;
mod media;
mod trends;
mod player;
mod qbit;
mod qsetup;
mod scan;
mod schedule;
mod search;
mod watch;
mod ytdl;

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
    /// Téléchargements directs (liens http)
    direct: direct::Direct,
    /// Épisodes vus / en cours (historique de VLC)
    watched: watch::Watched,
    cfg: Mutex<Config>,
    art: Artwork,
    schedules: schedule::Schedules,
    /// Torrents déjà vus par la tâche de fond (None = premier passage pas encore fait)
    seen: Mutex<Option<HashSet<String>>>,
    /// Séries pour lesquelles le mode séquentiel a déjà été réglé
    seq_done: Mutex<HashSet<String>>,
    /// Packs d'épisodes : hash → fichier mis en priorité maximale (-1 = aucun encore)
    boosted: Mutex<HashMap<String, i64>>,
    /// Épisodes cochés mais en attente (priorité 0 dans qBittorrent) : hash → fichier → infos
    held: Mutex<HashMap<String, HashMap<i64, Held>>>,
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
    fn save_held(&self) {
        let map = self.held.lock().unwrap().clone();
        let _ = std::fs::write(config::data_dir().join("held.json"), serde_json::to_vec(&map).unwrap_or_default());
    }
}

/// Épisode en attente de son tour : priorité voulue, taille et octets déjà reçus (figés pendant l'attente)
#[derive(Serialize, serde::Deserialize, Clone, Copy, PartialEq)]
struct Held {
    prio: i64,
    size: i64,
    done: i64,
}

fn load_held() -> HashMap<String, HashMap<i64, Held>> {
    std::fs::read(config::data_dir().join("held.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Packs d'épisodes : UN seul épisode téléchargé à la fois, dans l'ordre (01, puis 02…).
/// Une priorité « haute » dans qBittorrent n'est qu'une préférence : les autres fichiers cochés
/// avancent quand même. Donc les suivants passent à 0 dans qBittorrent (« en attente » ici) et
/// démarrent quand le précédent est fini.
/// Renvoie vrai si un épisode en attente vient d'être relancé.
async fn enforce_order(st: &AppState, hash: &str, files: &[TorrentFile]) -> bool {
    let mut media: Vec<&TorrentFile> = files.iter().filter(|f| is_media(&f.name)).collect();
    if media.len() < 2 {
        return false;
    }
    media.sort_by(|a, b| nat_cmp(&a.name, &b.name));
    let held = st.held.lock().unwrap().get(hash).cloned().unwrap_or_default();
    let prio_of = |f: &TorrentFile| if f.priority > 0 { f.priority } else { held.get(&f.index).map(|h| h.prio).unwrap_or(0) };
    let pending: Vec<&TorrentFile> = media.iter().copied().filter(|f| f.progress < 1.0 && prio_of(f) > 0).collect();
    // Épisode en cours : celui mis en priorité haute à la main, sinon le premier dans l'ordre
    let active = pending.iter().find(|f| prio_of(f) >= 6).or(pending.first()).copied();
    let mut next_held = HashMap::new();
    let mut to_hold = Vec::new();
    for f in &pending {
        if Some(f.index) == active.map(|a| a.index) {
            continue;
        }
        let keep = held.get(&f.index).copied();
        next_held.insert(f.index, keep.unwrap_or(Held { prio: prio_of(f), size: f.size, done: (f.size as f64 * f.progress) as i64 }));
        if f.priority > 0 {
            to_hold.push(f.index);
        }
    }
    if next_held != held {
        let mut all = st.held.lock().unwrap();
        if next_held.is_empty() {
            all.remove(hash);
        } else {
            all.insert(hash.to_string(), next_held);
        }
        drop(all);
        st.save_held();
    }
    if !to_hold.is_empty() {
        let _ = st.qbit.set_file_priority(hash, &to_hold, 0).await;
    }
    if let Some(a) = active.filter(|a| a.priority == 0) {
        return st.qbit.set_file_priority(hash, &[a.index], held.get(&a.index).map(|h| h.prio).unwrap_or(1)).await.is_ok();
    }
    false
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
    /// Type deviné d'après les vidéos, quand le NOM ne dit rien (« Ao no Hako » → série)
    fallback: Option<Kind>,
}

/// Numéro d'épisode dans un nom de fichier : « 01. Titre », « Titre - 12 », « E05 », « Ep 3 »
static RE_EP_NUM: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)(^\d{1,3}\b|[\s._-]\d{1,3}(?:v\d)?$|\b(?:e|ep|episode)\s?\d{1,4}\b|\s-\s\d{1,4}\b)").unwrap());

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
        return ContentInfo { kind: Some(Kind::Game), format: Some(ext.to_uppercase()), platform: Some(platform.into()), fallback: None };
    }
    let (book, audio, video) = (share(BOOK_EXT), share(AUDIO_EXT), share(VIDEO_EXT));
    let kind = if book > 0.5 {
        Some(Kind::Book)
    } else if audio > 0.5 && video < 0.1 {
        Some(Kind::Audio)
    } else {
        None
    };
    // Plusieurs vidéos numérotées (« 01. Titre.mkv ») = épisodes ; une seule grosse vidéo = film
    let videos: Vec<&TorrentFile> = files.iter().filter(|f| is_video(&f.name) && f.size > 50 * 1024 * 1024).collect();
    let numbered = videos.iter().filter(|f| RE_EP_NUM.is_match(Path::new(&f.name).file_stem().map(|s| s.to_string_lossy()).as_deref().unwrap_or(""))).count();
    let fallback = if video < 0.5 {
        None
    } else if videos.len() >= 2 && numbered * 2 > videos.len() {
        Some(Kind::Series)
    } else if videos.len() == 1 {
        Some(Kind::Film)
    } else {
        None
    };
    ContentInfo { kind, fallback, ..Default::default() }
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
    {
        let all = state.held.lock().unwrap();
        for m in media.iter_mut() {
            let Some(held) = all.get(&m.hash) else { continue };
            let (size, done) = held.values().fold((0, 0), |(s, d), h| (s + h.size, d + h.done));
            m.size += size;
            m.completed += done;
            m.progress = if m.size > 0 { m.completed as f64 / m.size as f64 } else { m.progress };
            if m.dlspeed > 0 {
                m.eta = (m.size - m.completed).max(0) / m.dlspeed;
            }
            if let Some(n) = m.files_sel.as_mut() {
                *n += held.len() as u32;
            }
        }
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
        } else if let Some(k) = info.fallback.filter(|_| m.kind == Kind::Other) {
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
    let mut files = state.qbit.files(&hash).await?;
    if let Some(held) = state.held.lock().unwrap().get(&hash) {
        for f in files.iter_mut().filter(|f| f.priority == 0) {
            if let Some(h) = held.get(&f.index) {
                f.priority = h.prio;
                f.queued = true;
            }
        }
    }
    Ok(files)
}

#[tauri::command]
async fn set_file_priority(state: State<'_, AppState>, hash: String, ids: Vec<i64>, priority: i64) -> Result<(), String> {
    // Épisode en attente : décoché → oublié ; autre priorité → retenue pour son tour
    let mut ids = ids;
    {
        let mut all = state.held.lock().unwrap();
        if let Some(held) = all.get_mut(&hash) {
            if priority == 0 {
                held.retain(|i, _| !ids.contains(i));
            } else {
                for i in &ids {
                    if let Some(h) = held.get_mut(i) {
                        h.prio = priority;
                    }
                }
                if priority < 6 {
                    ids.retain(|i| !held.contains_key(i));
                }
            }
            if held.is_empty() {
                all.remove(&hash);
            }
        }
    }
    state.save_held();
    if !ids.is_empty() {
        state.qbit.set_file_priority(&hash, &ids, priority).await?;
    }
    // Pack d'épisodes : on remet tout de suite dans l'ordre (sinon les épisodes cochés partent en même temps)
    if priority > 0 && state.boosted.lock().unwrap().contains_key(&hash) {
        if let Ok(files) = state.qbit.files(&hash).await {
            enforce_order(&state, &hash, &files).await;
        }
    }
    state.file_counts.lock().unwrap().remove(&hash);
    Ok(())
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
        "c411" => {
            let cfg = state.cfg();
            let mut tmdb_key = cfg.tmdb_key.clone();
            if tmdb_key.is_empty() {
                tmdb_key = option_env!("ONEFORALL_TMDB_KEY").unwrap_or_default().to_string();
            }
            // Pour TMDB, on enlève le numéro d'épisode (« S06E09 », « - 09 »)
            let title_only = regex::Regex::new(r"(?i)\s*(\bS\d{1,2}E\d{1,4}\b|\s-?\s*\d{1,4}\s*$)").unwrap().replace_all(query.trim(), "").to_string();
            let ids = search::tmdb_ids(&client, &tmdb_key, title_only.trim()).await;
            search::torznab(&client, "https://c411.org", &cfg.c411_key, query.trim(), "c411", ids).await
        }
        "online-fix" => search::online_fix(&client, query.trim()).await,
        "fitgirl" => search::fitgirl(&client, query.trim()).await,
        "nxbrew" => search::nxbrew(&client, query.trim()).await,
        "romsfun" => search::romsfun(&client, query.trim()).await,
        "wowroms" => search::wowroms(&client, query.trim()).await,
        _ => search::nyaa(&client, query.trim()).await,
    }
}

/// Tendances du moment pour un genre (TMDB, AniList, IGDB) : écran d'accueil de la recherche.
#[tauri::command]
async fn trends_catalog(state: State<'_, AppState>, kind: String) -> Result<Vec<trends::Trend>, String> {
    let mut tmdb = state.cfg().tmdb_key;
    if tmdb.is_empty() {
        tmdb = option_env!("ONEFORALL_TMDB_KEY").unwrap_or_default().to_string();
    }
    let client = reqwest::Client::builder().user_agent("OneForAll/0.7").timeout(Duration::from_secs(15)).build().map_err(|e| e.to_string())?;
    Ok(trends::cached(&format!("cat:{kind}"), trends::catalog(&client, &state.art, &kind, &tmdb)).await)
}

/// Ce qui est populaire en ce moment sur un site (C411, Nyaa, FitGirl, Online-Fix, RomsFun).
#[tauri::command]
async fn site_popular(state: State<'_, AppState>, source: String, genre: String) -> Result<Vec<search::SearchResult>, String> {
    let key = state.cfg().c411_key;
    let client = reqwest::Client::builder().user_agent("OneForAll/0.7").timeout(Duration::from_secs(15)).build().map_err(|e| e.to_string())?;
    let mut err = None;
    let list = trends::cached(&format!("site:{source}:{genre}"), async {
        search::popular(&client, &source, &genre, &key).await.unwrap_or_else(|e| {
            err = Some(e);
            Vec::new()
        })
    })
    .await;
    match err {
        Some(e) if list.is_empty() => Err(e),
        _ => Ok(list),
    }
}

/// Versions d'un jeu RomsFun (Europe, USA…).
#[tauri::command]
async fn rom_versions(page: String) -> Result<Vec<search::RomVersion>, String> {
    if !page.starts_with("https://romsfun.com/") {
        return Err("Lien RomsFun invalide".into());
    }
    let client = reqwest::Client::builder().user_agent("OneForAll/0.6").timeout(Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
    search::romsfun_versions(&client, &page).await
}

/// Ajoute une version RomsFun à l'onglet DL (lien direct de la page).
#[tauri::command]
async fn rom_download(state: State<'_, AppState>, page: String, title: String, platform: String, thumb: String) -> Result<String, String> {
    if !page.starts_with("https://romsfun.com/download/") {
        return Err("Lien RomsFun invalide".into());
    }
    let client = reqwest::Client::builder().user_agent("OneForAll/0.6").timeout(Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
    let link = search::romsfun_link(&client, &page).await?;
    Ok(state.direct.add_game(&link, &title, &platform, &thumb))
}

/// Liens d'hébergeurs d'une page NXBrew (jeu, mise à jour, DLC).
#[tauri::command]
async fn nxbrew_links(page: String) -> Result<Vec<search::NxSection>, String> {
    if !page.starts_with("https://nxbrew.net/") {
        return Err("Lien NXBrew invalide".into());
    }
    let client = reqwest::Client::builder().user_agent("OneForAll/0.6").timeout(Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
    let res = client.get(&page).send().await.map_err(|e| format!("nxbrew.net injoignable : {e}"))?;
    if !res.status().is_success() {
        return Err(format!("nxbrew.net a répondu {}", res.status()));
    }
    let html = res.text().await.map_err(|e| e.to_string())?;
    Ok(search::nxbrew_sections(&html))
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
    /// Liens ajoutés comme téléchargements directs (onglet Téléchargements)
    direct: Vec<String>,
}

/// Un lien http mène-t-il (peut-être) à un .torrent ? Sinon, inutile de télécharger la réponse pour vérifier.
fn looks_like_torrent(url: &str, r: &reqwest::Response) -> bool {
    let h = |k: reqwest::header::HeaderName| r.headers().get(k).map(|v| String::from_utf8_lossy(v.as_bytes()).to_lowercase()).unwrap_or_default();
    let ctype = h(reqwest::header::CONTENT_TYPE);
    if ctype.contains("bittorrent") || h(reqwest::header::CONTENT_DISPOSITION).contains(".torrent") || url.to_lowercase().split('?').next().unwrap_or("").ends_with(".torrent") {
        return true;
    }
    // Petit fichier binaire sans indication : on le lit pour vérifier (un .torrent pèse rarement plus de 10 Mo)
    !ctype.starts_with("text/html") && r.content_length().is_some_and(|l| l < 10 * 1024 * 1024)
}

// ---------- téléchargements directs ----------

#[derive(Serialize)]
struct DirectView {
    /// Console (jeu d'un site de ROM) et ROM extraite de l'archive
    platform: String,
    rom: String,
    id: String,
    url: String,
    name: String,
    title: String,
    kind: Kind,
    art_key: String,
    year: Option<u32>,
    season: Option<u32>,
    episode: Option<u32>,
    dir: String,
    path: String,
    size: i64,
    done: i64,
    progress: f64,
    speed: i64,
    eta: i64,
    /// queued, running, paused, done, error
    st: direct::St,
    retrying: bool,
    error: String,
    resumable: bool,
    conns: usize,
    added_on: i64,
    completion_on: i64,
    /// Vidéo / musique d'un site (yt-dlp) : format (max, 1080, 720, mp3) et son libellé
    media: String,
    media_label: String,
    /// Extrait : « 1m00 → 2m00 »
    section_label: String,
    thumb: String,
    /// Étape en cours (installation de yt-dlp / ffmpeg, analyse, finalisation…)
    note: String,
}

#[tauri::command]
fn get_downloads(state: State<'_, AppState>) -> Vec<DirectView> {
    let split = state.cfg().split_anime;
    // ROM terminées (téléchargées hors site de ROM) : reconnues une fois, en arrière-plan
    let romish = |d: &direct::Dl| {
        let p = d.final_path();
        emu::is_rom(&p) || emu::is_archive(&p)
    };
    let todo = state.direct.take_unchecked(romish);
    if !todo.is_empty() {
        let direct = state.direct.clone();
        std::thread::spawn(move || {
            for d in todo {
                if let Some(m) = emu::detect(&d.final_path()) {
                    let stem = d.final_path().file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    // « Mega Man - Maverick Hunter X (Europe) » → « Mega Man - Maverick Hunter X »
                    let title = regex::Regex::new(r"s*[([][^)]]*[)]]").unwrap().replace_all(&stem.replace('_', " "), "").trim().to_string();
                    direct.set_game(&d.id, if title.is_empty() { &stem } else { &title }, emu::platform_of(m.id));
                }
            }
        });
    }
    let mut out: Vec<DirectView> = state
        .direct
        .list()
        .into_iter()
        .map(|d| {
            let raw = d.raw();
            let mut m = Media::from_raw(raw.clone());
            if m.kind == Kind::Anime && !split {
                m.kind = Kind::Series;
            }
            // Vidéo / musique d'un site : son titre tel quel, sa miniature (pas d'affiche de film à chercher)
            if !d.media.is_empty() {
                m.kind = if d.media == "mp3" { Kind::Audio } else { Kind::Other };
                m.title = match raw.name.rsplit_once('.') {
                    Some((stem, ext)) if d.st == direct::St::Done && ext.len() <= 4 => stem.to_string(),
                    _ if d.name.is_empty() => String::new(),
                    _ => raw.name.clone(),
                };
            }
            // Jeu d'un site de ROM : son vrai titre (pas le nom du .zip), donc la bonne jaquette
            if !d.game.is_empty() && (d.kind.is_empty() || d.kind == "game") {
                m.kind = Kind::Game;
                m.title = d.game.clone();
                m.group_key = format!("game:{}", media::normalize(&d.game));
            }
            // Type choisi à la main
            if let Some(k) = Kind::parse(&d.kind) {
                if k != m.kind {
                    let rest = m.group_key.split_once(':').map(|(_, r)| r.to_string()).unwrap_or_default();
                    m.group_key = format!("{}:{rest}", if matches!(k, Kind::Series | Kind::Anime) { "show" } else { k.as_str() });
                    m.kind = k;
                }
            }
            DirectView {
                platform: d.platform.clone(),
                rom: d.rom.clone(),
                media: d.media.clone(),
                media_label: match d.media.as_str() {
                    "" => String::new(),
                    "compress" => "Compressé".into(),
                    "clip" => "Extrait".into(),
                    "mp3" => "MP3".into(),
                    m if !d.res.is_empty() && m != "mp3" => d.res.clone(),
                    m => ytdl::label(m).into(),
                },
                section_label: d.section().map(|(a, b)| format!("{} → {}", ytdl::hms(a), ytdl::hms(b))).unwrap_or_default(),
                thumb: d.thumb.clone(),
                note: d.note.clone(),
                conns: if d.st == direct::St::Running { d.segs.iter().filter(|s| s.end < 0 || s.start + s.done < s.end).count() } else { 0 },
                id: d.id,
                url: d.url,
                name: raw.name,
                title: m.title,
                kind: m.kind,
                art_key: m.group_key,
                year: m.year,
                season: m.season,
                episode: m.episode,
                dir: d.dir,
                path: raw.content_path,
                size: raw.size,
                done: raw.completed,
                progress: raw.progress,
                speed: raw.dlspeed,
                eta: raw.eta,
                st: d.st,
                retrying: d.retrying,
                error: d.error,
                resumable: d.resumable,
                added_on: d.added_on,
                completion_on: d.completion_on,
            }
        })
        .collect();
    out.sort_by(|a, b| b.added_on.cmp(&a.added_on));
    out
}

/// Bouton de la fenêtre « Épisode suivant » : « next » ou « cancel ».
#[tauri::command]
fn overlay_action(action: String) {
    player::overlay_action(&action);
}

/// Ce qu'affiche la fenêtre « Épisode suivant » (épisode, série, affiche, durée du compte à rebours).
#[tauri::command]
fn overlay_data() -> Option<serde_json::Value> {
    player::overlay_data()
}

/// Vu / en cours pour chaque fichier (clé = chemin demandé).
#[tauri::command]
fn get_watch(state: State<'_, AppState>, paths: Vec<String>) -> HashMap<String, watch::Watch> {
    state.watched.get(&paths)
}

#[tauri::command]
fn set_watched(state: State<'_, AppState>, paths: Vec<String>, watched: bool) {
    state.watched.set(&paths, watched);
}

/// `format` : max, 1080, 720, mp3 (vidéos des sites) ; `section` : « début-fin » en secondes pour un extrait
#[tauri::command]
fn add_downloads(state: State<'_, AppState>, urls: Vec<String>, format: Option<String>, section: Option<String>) -> Vec<String> {
    let fmt = format.unwrap_or_else(|| "max".into());
    let section = section.unwrap_or_default();
    urls.iter()
        .map(|u| u.trim())
        .filter(|u| u.starts_with("http"))
        .map(|u| {
            if ytdl::is_spotify(u) {
                state.direct.add_media(u, "mp3")
            } else if ytdl::is_media_url(u) {
                let id = state.direct.add_media(u, &fmt);
                if !section.is_empty() && !ytdl::is_playlist_url(u) {
                    state.direct.set_section(&id, &section);
                }
                id
            } else {
                state.direct.add(u)
            }
        })
        .collect()
}


/// Titre, durée, aperçu lisible et définitions dispo d'une vidéo (timeline de l'extrait, choix du format)
#[tauri::command]
async fn media_info(url: String) -> Result<ytdl::Info, String> {
    let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
    let exe = ytdl::ytdlp(&client, &|_| {}).await?;
    let mut i = ytdl::info(&exe, &url).await?;
    ytdl::expose_preview(&mut i);
    Ok(i)
}

#[derive(Serialize)]
struct CompressInfo {
    probe: ytdl::Probe,
    /// (libellé, taille visée en octets, hauteur de sortie)
    levels: Vec<(String, i64, i64)>,
}

/// Niveaux de compression proposés pour une vidéo, avec la taille qu'elle fera
#[tauri::command]
async fn compress_info(path: String) -> Result<CompressInfo, String> {
    let client = reqwest::Client::new();
    let ff = ytdl::ffmpeg(&client, &|_| {}).await?;
    let p = tauri::async_runtime::spawn_blocking(move || ytdl::probe(&ff, Path::new(&path))).await.map_err(|e| e.to_string())??;
    if p.duration <= 0.0 {
        return Err("Durée de la vidéo inconnue".into());
    }
    let mb = 1_048_576i64;
    let mut levels: Vec<(String, i64)> = vec![
        ("Léger".into(), p.size * 60 / 100),
        ("Moyen".into(), p.size * 35 / 100),
        ("Fort".into(), p.size * 15 / 100),
    ];
    // Limites courantes de partage
    for (label, size) in [("Discord", 10 * mb), ("WhatsApp", 16 * mb), ("Mail", 25 * mb)] {
        if size < p.size * 9 / 10 {
            levels.push((label.into(), size * 95 / 100));
        }
    }
    // En dessous de ~100 kbit/s d'image, c'est illisible
    let floor = ((100_000 + p.audio.min(64_000)) as f64 * p.duration / 8.0) as i64;
    levels.retain(|(_, s)| *s >= floor);
    levels.sort_by(|a, b| b.1.cmp(&a.1));
    let levels = levels.into_iter().map(|(l, s)| {
        let h = ytdl::plan(&p, s).1;
        (l, s, h)
    }).collect();
    Ok(CompressInfo { probe: p, levels })
}

#[tauri::command]
fn compress_video(state: State<'_, AppState>, path: String, target: i64) -> String {
    state.direct.add_compress(&path, target)
}

/// Studio d'extraits : pistes, durée et adresse de l'aperçu d'une vidéo locale
#[tauri::command]
async fn clip_open(path: String) -> Result<clip::Source, String> {
    let ff = ytdl::ffmpeg(&reqwest::Client::new(), &|_| {}).await?;
    let src = tauri::async_runtime::spawn_blocking(move || clip::open(&ff, &path)).await.map_err(|e| e.to_string())??;
    clip::warm(&src);
    Ok(src)
}


/// Courbe du son d'une piste (studio), 50 points par seconde
#[tauri::command]
async fn clip_wave(id: String, audio: usize) -> Result<Vec<u8>, String> {
    let src = clip::source(&id).ok_or("Vidéo fermée")?;
    tauri::async_runtime::spawn_blocking(move || clip::wave(&src, audio)).await.map_err(|e| e.to_string())
}

/// Musique ou image ajoutée dans le studio
#[tauri::command]
async fn clip_asset(path: String) -> Result<clip::Asset, String> {
    let ff = ytdl::ffmpeg(&reqwest::Client::new(), &|_| {}).await?;
    tauri::async_runtime::spawn_blocking(move || clip::asset(&ff, &path)).await.map_err(|e| e.to_string())?
}

/// Courbe du son d'une vidéo en ligne (timeline « Un passage »)
#[tauri::command]
async fn media_wave(id: String) -> Result<Vec<u8>, String> {
    let ff = ytdl::ffmpeg(&reqwest::Client::new(), &|_| {}).await?;
    tauri::async_runtime::spawn_blocking(move || ytdl::wave(&ff, &id)).await.map_err(|e| e.to_string())
}
/// Avancement de la copie de lecture rapide du studio
#[tauri::command]
fn clip_proxy(id: String) -> Option<clip::Proxy> {
    clip::proxy(&id)
}

/// Exporte les passages choisis (montage) en MP4 dans le dossier des extraits ; apparaît dans l'onglet DL
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn clip_export(
    state: State<'_, AppState>,
    path: String,
    segs: Vec<clip::Seg>,
    height: i64,
    name: Option<String>,
    layers: Option<Vec<clip::Layer>>,
    music: Option<Vec<clip::Music>>,
    volume: Option<f64>,
) -> Result<String, String> {
    if segs.is_empty() {
        return Err("Aucun passage".into());
    }
    let dir = state.cfg().folders.get("clips").filter(|f| !f.trim().is_empty()).cloned().unwrap_or_else(|| {
        let home = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into());
        Path::new(&home).join("Videos").join("OneForAll Extraits").display().to_string()
    });
    let stem = Path::new(&path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "extrait".into());
    let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| {
        // « Épisode [12m30-13m10].mp4 »
        let (a, b) = (segs[0].start, segs.last().map(|s| s.end).unwrap_or(0.0));
        format!("{stem} [{}-{}]", clip::stamp(a), clip::stamp(b))
    });
    let name = if name.to_lowercase().ends_with(".mp4") { name } else { format!("{name}.mp4") };
    let name: String = name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    let spec = clip::Spec { path, segs, height, layers: layers.unwrap_or_default(), music: music.unwrap_or_default(), volume: volume.unwrap_or(1.0) };
    Ok(state.direct.add_clip(&spec, &dir, &name))
}
#[tauri::command]
async fn download_action(state: State<'_, AppState>, ids: Vec<String>, action: String) -> Result<(), String> {
    state.direct.action(&ids, &action).await
}

#[tauri::command]
async fn move_downloads(state: State<'_, AppState>, ids: Vec<String>, dir: String) -> Result<(), String> {
    state.direct.set_dir(&ids, &dir).await
}

/// Ajoute des torrents (liens magnet, liens .torrent, fichiers .torrent) sans les démarrer.
async fn stage(st: &AppState, urls: &[String], paths: &[String]) -> Staged {
    let mut out = Staged { hashes: Vec::new(), errors: Vec::new(), direct: Vec::new() };
    let add_bytes = |name: String, bytes: Vec<u8>| async move { (scan::infohash(&bytes), st.qbit.add_file(&name, bytes, true).await) };
    for u in urls.iter().map(|u| u.trim()).filter(|u| !u.is_empty()) {
        if u.to_lowercase().starts_with("magnet:") {
            match st.qbit.add_magnet_staged(u).await {
                Ok(()) => out.hashes.extend(scan::magnet_hash(u)),
                Err(e) => out.errors.push(e),
            }
        } else if ytdl::is_media_url(u) {
            // YouTube, X, Insta, TikTok… : directement dans l'onglet DL, meilleure qualité
            out.direct.push(st.direct.add_media(u, if ytdl::is_spotify(u) { "mp3" } else { "max" }));
        } else if u.starts_with("http") {
            // Lien d'un fichier .torrent (ex. nyaa.si) : on le télécharge pour connaître son contenu
            let client = reqwest::Client::builder().user_agent("OneForAll/0.5").timeout(Duration::from_secs(20)).build();
            let res = match client {
                Ok(c) => c.get(u).send().await.map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            match res {
                // Gros fichier ou page qui n'est pas un .torrent : c'est un téléchargement direct
                Ok(r) if r.status().is_success() && !looks_like_torrent(u, &r) => {
                    drop(r);
                    out.direct.push(st.direct.add(u));
                }
                Ok(r) if r.status().is_success() => match r.bytes().await {
                    Ok(b) if scan::torrent_files(&b).is_none() && !u.to_lowercase().split('?').next().unwrap_or("").ends_with(".torrent") => {
                        out.direct.push(st.direct.add(u));
                    }
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

// ---------- un dossier par série ----------

/// Nom de dossier d'une série : la variante du titre avec apostrophe si elle existe (« JoJo's » plutôt que « JoJos »).
fn show_folder_name(titles: &[&str]) -> String {
    let best = titles.iter().find(|t| t.contains('\'')).or(titles.first()).copied().unwrap_or("Série");
    safe_name(best).trim_end_matches('.').to_string()
}

fn same_dir(a: &str, b: &str) -> bool {
    let n = |s: &str| s.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    n(a) == n(b)
}

/// Dossier cible d'un torrent de série : « <base>\<Titre> » (base = dossier actuel, sans le dossier de série s'il y est déjà).
fn show_target(save_path: &str, folder: &str) -> String {
    let p = Path::new(save_path.trim_end_matches(['\\', '/']));
    let base = if p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(folder)) { p.parent().unwrap_or(p) } else { p };
    base.join(folder).display().to_string()
}

/// Range chaque série (épisodes en torrents séparés) dans son propre dossier. Renvoie le nombre de torrents déplacés.
#[tauri::command]
async fn organize_shows(state: State<'_, AppState>, group_key: Option<String>) -> Result<usize, String> {
    let media = get_torrents(state.clone()).await?;
    let mut groups: HashMap<String, Vec<&Media>> = HashMap::new();
    for m in media.iter().filter(|m| matches!(m.kind, Kind::Series | Kind::Anime) && m.status != media::Status::Error) {
        if group_key.as_ref().is_none_or(|k| *k == m.group_key) {
            groups.entry(m.group_key.clone()).or_default().push(m);
        }
    }
    let mut moved = 0;
    for items in groups.values() {
        // Une série en un seul torrent-dossier est déjà regroupée
        let single_files = items.iter().filter(|m| Path::new(&m.content_path).is_file()).count();
        if items.len() < 2 && single_files == 0 {
            continue;
        }
        let titles: Vec<&str> = items.iter().map(|m| m.title.as_str()).collect();
        let folder = show_folder_name(&titles);
        for m in items {
            let target = show_target(&m.save_path, &folder);
            if !same_dir(&m.save_path, &target) {
                state.qbit.set_location(&[m.hash.clone()], &target).await?;
                moved += 1;
            }
        }
    }
    Ok(moved)
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
    let held: HashSet<i64> = state.held.lock().unwrap().get(&hash).map(|h| h.keys().copied().collect()).unwrap_or_default();
    let wanted: HashSet<i64> = ids.into_iter().filter(|i| !held.contains(i)).collect();
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
    if !Path::new(&path).exists() {
        return Err("Ce fichier n'existe pas (encore).".into());
    }
    match shell_open(&path) {
        Ok(()) => Ok(()),
        // Aucune appli associée (ex. .mkv sans lecteur par défaut) : VLC s'il est là
        Err(e) => match find_vlc() {
            Some(vlc) => std::process::Command::new(vlc).arg(&path).spawn().map(|_| ()).map_err(|e| e.to_string()),
            None => Err(e),
        },
    }
}

/// Ouvre un fichier, un dossier ou un lien comme un double-clic Windows.
/// (explorer.exe ignore certains liens avec « & » et certains noms avec « ： » ou 『』, sans rien dire)
fn shell_open(target: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        let wide = |s: &str| std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect::<Vec<u16>>();
        let (verb, file) = (wide("open"), wide(target));
        let r = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), 1) } as isize;
        if r > 32 {
            Ok(())
        } else if r == 31 {
            Err("Aucune application n'est associée à ce type de fichier.".into())
        } else {
            Err(format!("Ouverture impossible (code {r})"))
        }
    }
    #[cfg(not(windows))]
    explorer(format!("\"{target}\""))
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
async fn play(app: tauri::AppHandle, state: State<'_, AppState>, hashes: Vec<String>) -> Result<usize, String> {
    let ts = state.qbit.torrents().await?;
    let mut hashes = hashes;
    // Un seul épisode d'une série en plusieurs torrents : les épisodes suivants déjà téléchargés
    // passent à la suite (pour la fenêtre « Épisode suivant »)
    if let [h] = hashes.as_slice() {
        if let Some(me) = ts.iter().find(|t| &t.hash == h).map(|t| Media::from_raw(t.clone())) {
            if let Some(ep) = me.episode {
                let order = |m: &Media| (m.season.unwrap_or(0), m.episode.unwrap_or(0));
                let mut next: Vec<(u32, u32, String)> = ts
                    .iter()
                    .filter(|t| t.hash != me.hash && t.progress >= 0.999)
                    .map(|t| Media::from_raw(t.clone()))
                    .filter(|m| m.group_key == me.group_key && m.episode.is_some() && order(m) > (me.season.unwrap_or(0), ep))
                    .map(|m| (m.season.unwrap_or(0), m.episode.unwrap_or(0), m.hash.clone()))
                    .collect();
                next.sort();
                next.dedup_by_key(|x| (x.0, x.1));
                hashes.extend(next.into_iter().map(|x| x.2));
            }
        }
    }
    let mut complete = Vec::new();
    let mut partial = Vec::new();
    for h in &hashes {
        let Some(t) = ts.iter().find(|t| &t.hash == h) else { continue };
        let show = show_info(&state, t).await;
        let mut files = state.qbit.files(h).await?;
        files.retain(|f| f.priority > 0 && f.progress > 0.0 && is_media(&f.name));
        files.sort_by(|a, b| nat_cmp(&a.name, &b.name));
        for f in files {
            let p = file_path(&t.save_path, &f);
            if p.is_file() {
                let item = play_item(&show, p);
                if f.progress >= 0.999 { complete.push(item) } else { partial.push(item) }
            }
        }
    }
    // On lit ce qui est complet ; à défaut, le début de ce qui est en cours.
    let list = if complete.is_empty() { partial } else { complete };
    start_player(app, list)
}

/// Lit des fichiers précis d'un torrent (liste des fichiers d'un pack : « Reprendre », double-clic…),
/// suivis des épisodes suivants déjà téléchargés.
#[tauri::command]
async fn play_files(app: tauri::AppHandle, state: State<'_, AppState>, hash: String, paths: Vec<String>) -> Result<usize, String> {
    let ts = state.qbit.torrents().await?;
    let t = ts.iter().find(|t| t.hash == hash).ok_or("Torrent introuvable")?;
    let show = show_info(&state, t).await;
    let list = paths.into_iter().map(PathBuf::from).filter(|p| p.is_file()).map(|p| play_item(&show, p)).collect();
    start_player(app, list)
}

/// Série d'un torrent : clé (pistes préférées), titre, affiche — pour la fenêtre « Épisode suivant »
struct ShowInfo {
    key: String,
    title: String,
    art: Option<String>,
}

async fn show_info(state: &AppState, t: &qbit::RawTorrent) -> ShowInfo {
    let m = Media::from_raw(t.clone());
    let mut tmdb = state.cfg().tmdb_key;
    if tmdb.is_empty() {
        tmdb = option_env!("ONEFORALL_TMDB_KEY").unwrap_or_default().to_string();
    }
    let art = state.art.get(&m.group_key, m.kind.as_str(), &m.title, m.year, &tmdb).await.map(|a| a.url);
    ShowInfo { key: m.group_key, title: m.title, art }
}

fn play_item(show: &ShowInfo, path: PathBuf) -> player::Item {
    // Numéro d'épisode d'après le nom du fichier (« S01E05 », « - 05 », « 05. Titre »)
    let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ep = Media::from_raw(qbit::RawTorrent { name, ..Default::default() });
    let label = match (ep.season, ep.episode) {
        (Some(s), Some(e)) => format!("S{s} · É{e}"),
        (None, Some(e)) => format!("Épisode {e}"),
        _ => String::new(),
    };
    player::Item { path, series: Some(show.key.clone()), label, title: show.title.clone(), art: show.art.clone(), episode: ep.episode }
}

fn start_player(app: tauri::AppHandle, list: Vec<player::Item>) -> Result<usize, String> {
    if list.is_empty() {
        return Err("Aucune vidéo ou musique téléchargée à lire.".into());
    }
    let n = list.len();
    match find_vlc() {
        // VLC suivi : position exacte (vu / en cours) et pistes audio / sous-titres retenues par série
        Some(vlc) => player::launch(app, &vlc, list)?,
        None => explorer(format!("\"{}\"", list[0].path.display()))?,
    }
    Ok(n)
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
        c.seq_episodes = settings.seq_episodes;
        c.tmdb_key = settings.tmdb_key.trim().to_string();
        c.split_anime = settings.split_anime;
        c.c411_key = settings.c411_key.trim().to_string();
        c.clone()
    };
    config::save(&cfg)?;
    state.seq_done.lock().unwrap().clear();
    state.direct.set_folders(cfg.folders.clone(), cfg.split_anime);
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

/// Calendrier de diffusion d'une série (prochain épisode, épisodes sortis).
#[tauri::command]
async fn get_schedule(state: State<'_, AppState>, titles: Vec<String>, kind: String) -> Result<Option<schedule::Schedule>, String> {
    Ok(state.schedules.get(&titles, &kind).await)
}

#[tauri::command]
fn forget_artwork(state: State<'_, AppState>, key: String) {
    state.art.forget(&key);
}

/// Images proposées pour un titre (fenêtre « Choisir l'image »).
#[tauri::command]
async fn art_choices(state: State<'_, AppState>, kind: String, title: String, year: Option<u32>) -> Result<Vec<artwork::Choice>, String> {
    let mut tmdb = state.cfg().tmdb_key;
    if tmdb.is_empty() {
        tmdb = option_env!("ONEFORALL_TMDB_KEY").unwrap_or_default().to_string();
    }
    Ok(state.art.choices(&kind, &title, year, &tmdb).await)
}

/// Image choisie : une proposition (url), un fichier (path) ou une image collée (data, base64).
#[tauri::command]
fn set_artwork(state: State<'_, AppState>, key: String, url: Option<String>, fallback: Option<String>, path: Option<String>, data: Option<String>) -> Result<Art, String> {
    use base64::Engine;
    let art = if let Some(url) = url.filter(|u| u.starts_with("http")) {
        Art { url, fallback, kind: None }
    } else if let Some(path) = path {
        artwork::custom_image(&std::fs::read(&path).map_err(|e| format!("Lecture impossible : {e}"))?)?
    } else if let Some(data) = data {
        let b64 = data.split_once(",").map(|(_, b)| b).unwrap_or(&data);
        artwork::custom_image(&base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|_| "Image collée illisible".to_string())?)?
    } else {
        return Err("Aucune image".into());
    };
    // Le type corrigé auparavant (jeu, animé) est gardé
    let art = Art { kind: state.art.cached(&key).and_then(|a| a.kind), ..art };
    state.art.set(&key, art.clone());
    Ok(art)
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
fn notify_finished(app: &tauri::AppHandle, m: &Media, poster: Option<&Path>, hero: Option<&Path>) {
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
    if let Some(p) = hero {
        // Petite vignette à gauche (le bandeau « hero » prenait la moitié de l'écran)
        toast = toast.icon(p, IconCrop::Square, &m.title);
    }
    let _ = toast.show();
}

/// Miniature d'une vidéo en PNG local (les notifications Windows ne lisent ni le WebP ni les URL)
async fn thumb_file(id: &str, url: &str) -> Option<PathBuf> {
    if url.is_empty() {
        return None;
    }
    let path = config::data_dir().join("thumbs").join(format!("{id}-sq.png"));
    if path.is_file() {
        return Some(path);
    }
    let bytes = reqwest::Client::new().get(url).timeout(Duration::from_secs(10)).send().await.ok()?.bytes().await.ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
    // Vignette carrée (centre de l'image)
    let img = img.resize_to_fill(256, 256, image::imageops::FilterType::Triangle);
    std::fs::create_dir_all(path.parent()?).ok()?;
    img.save_with_format(&path, image::ImageFormat::Png).ok()?;
    Some(path)
}

/// Affiche une notification d'exemple avec le dernier torrent terminé.
#[tauri::command]
async fn test_notification(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let ts = state.qbit.torrents().await?;
    let t = ts.iter().filter(|t| t.progress >= 1.0).max_by_key(|t| t.completion_on).ok_or("Aucun torrent terminé")?;
    let m = Media::from_raw(t.clone());
    let poster = state.art.poster_file(&m.group_key).await;
    notify_finished(&app, &m, poster.as_deref(), None);
    Ok(())
}

async fn background_tick(st: &AppState, app: &tauri::AppHandle) {
    let qts = st.qbit.torrents().await;
    // Téléchargements directs compris (notification de fin), même si qBittorrent ne répond pas
    let mut all = qts.clone().unwrap_or_default();
    all.extend(st.direct.list().iter().map(|d| d.raw()));

    // 0. Téléchargements qui viennent de se terminer → notification Windows
    let finished: Vec<Media> = {
        let mut last = st.last_progress.lock().unwrap();
        // Pack dont des épisodes attendent leur tour : 100 % dans qBittorrent, mais pas fini
        let held: HashSet<String> = st.held.lock().unwrap().keys().cloned().collect();
        let mut out = Vec::new();
        for t in &all {
            let before = last.insert(t.hash.clone(), (t.progress, t.state.clone()));
            // Seulement si on l'a VU en train de télécharger juste avant. Au démarrage, qBittorrent
            // annonce d'abord ses torrents à 0 % (« vérification ») puis à 100 % : ce n'est pas une fin.
            let was_downloading = before.is_some_and(|(p, s)| {
                p < 0.999 && matches!(s.as_str(), "downloading" | "stalledDL" | "forcedDL" | "queuedDL")
            });
            if was_downloading && t.progress >= 0.999 && !held.contains(&t.hash) {
                out.push(Media::from_raw(t.clone()));
            }
        }
        out
    };
    for mut m in finished {
        // Vidéo / musique d'un site : sa miniature en grand, son vrai titre (pas d'affiche de film)
        let media = if direct::is_direct(&m.hash) { st.direct.get(&m.hash).filter(|d| !d.media.is_empty()) } else { None };
        if let Some(d) = media {
            m.title = Path::new(&d.name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or(d.name.clone());
            m.season = None;
            m.episode = None;
            let hero = thumb_file(&d.id, &d.thumb).await;
            notify_finished(app, &m, None, hero.as_deref());
            continue;
        }
        let poster = st.art.poster_file(&m.group_key).await;
        notify_finished(app, &m, poster.as_deref(), None);
    }
    let Ok(ts) = qts else { return };
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
            // Nom sans indice (« Ao no Hako ») : on regarde les fichiers
            if kind == "other" {
                if let Ok(files) = st.qbit.files(&t.hash).await {
                    let info = content_info(&files);
                    if let Some(k) = info.kind.or(info.fallback) {
                        kind = k.as_str();
                    }
                }
            }
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

                // Épisode de série : dans le dossier de la série (« D:\Series\JoJo's Bizarre Adventure\ »),
                // avec les autres épisodes déjà présents
                if kind == "series" || kind == "anime" {
                    let me = Media::from_raw(t.clone());
                    let siblings: Vec<Media> = ts.iter().filter(|o| o.hash != t.hash).map(|o| Media::from_raw(o.clone())).filter(|o| o.group_key == me.group_key).collect();
                    if !siblings.is_empty() || is_media(&t.name) {
                        let base = cfg
                            .folders
                            .get(kind)
                            .filter(|f| !f.is_empty())
                            .cloned()
                            .or_else(|| cats.get(cat).map(|c| c.save_path.clone()).filter(|p| !p.is_empty()))
                            .unwrap_or_else(|| t.save_path.clone());
                        let mut titles: Vec<&str> = siblings.iter().map(|o| o.title.as_str()).collect();
                        titles.push(&me.title);
                        let target = show_target(&base, &show_folder_name(&titles));
                        let _ = st.qbit.set_location(&h, &target).await;
                    }
                }
            }
        }
    }

    {
        let alive: HashSet<&str> = ts.iter().map(|t| t.hash.as_str()).collect();
        let mut all = st.held.lock().unwrap();
        let before = all.len();
        all.retain(|h, _| alive.contains(h.as_str()));
        let changed = all.len() != before;
        drop(all);
        if changed {
            st.save_held();
        }
    }

    // 2. Vidéos et audio en cours → TOUJOURS téléchargés dans l'ordre (épisode 1, puis 2…)
    let mut seq = Vec::new();
    let mut flpp_toggle = Vec::new();
    for t in &ts {
        let waiting = st.held.lock().unwrap().contains_key(&t.hash);
        // Épisode fini mais d'autres en attente : qBittorrent le croit terminé, on continue quand même
        if (t.progress >= 1.0 && !waiting) || !t.has_metadata || matches!(t.state.as_str(), "pausedDL" | "stoppedDL") {
            continue;
        }
        if !matches!(detect_kind(&t.name, &t.category), Kind::Series | Kind::Anime | Kind::Film | Kind::Audio) {
            continue;
        }
        let first_time = !st.seq_done.lock().unwrap().contains(&t.hash);
        let is_pack = st.boosted.lock().unwrap().contains_key(&t.hash);
        if !first_time && !is_pack {
            continue;
        }
        let Ok(files) = st.qbit.files(&t.hash).await else { continue };
        st.seq_done.lock().unwrap().insert(t.hash.clone());
        let mut media: Vec<&TorrentFile> = files.iter().filter(|f| is_media(&f.name)).collect();
        media.sort_by(|a, b| nat_cmp(&a.name, &b.name));
        let pack = media.len() >= 2;
        if first_time {
            if !t.seq_dl {
                seq.push(t.hash.clone());
            }
            // « Premier et dernier morceau d'abord » : utile pour lire tôt UN fichier, mais sur un pack
            // il entame le début et la fin de CHAQUE épisode → tous avancent en même temps.
            if t.f_l_piece_prio == pack {
                flpp_toggle.push(t.hash.clone());
            }
            if pack {
                st.boosted.lock().unwrap().insert(t.hash.clone(), -1);
            }
        }
        if pack {
            let released = enforce_order(st, &t.hash, &files).await;
            // Arrêté par qBittorrent à « 100 % » (limite de partage) alors qu'un épisode vient d'être relancé
            if released && matches!(t.state.as_str(), "pausedUP" | "stoppedUP") {
                let _ = st.qbit.action(&[t.hash.clone()], "resume").await;
            }
        }
    }
    if !seq.is_empty() {
        let _ = st.qbit.action(&seq, "toggle_seq").await;
    }
    if !flpp_toggle.is_empty() {
        let _ = st.qbit.action(&flpp_toggle, "toggle_flpp").await;
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
    shell_open(&url)
}

/// Ouvre la page Windows « Applications par défaut » de OneForAll (pour les liens magnet).
#[tauri::command]
fn open_default_apps() -> Result<(), String> {
    explorer("ms-settings:defaultapps?registeredAppUser=OneForAll".into())
}

// ---------- émulateurs ----------

#[tauri::command]
fn emu_list() -> Vec<emu::EmuInfo> {
    let map = emu::installed();
    emu::EMUS.iter().map(|m| emu::info(m, &map)).collect()
}

#[tauri::command]
fn emu_detect(path: String) -> emu::Rom {
    emu::rom_info(std::path::Path::new(&path))
}

/// Type d'un téléchargement choisi à la main (« » = automatique)
#[tauri::command]
fn dl_set_kind(state: State<'_, AppState>, id: String, kind: String) {
    state.direct.set_kind(&id, &kind);
}

/// Extrait l'archive d'un téléchargement (ROM .zip / .7z) à côté d'elle et retient la ROM.
#[tauri::command]
async fn dl_extract(state: State<'_, AppState>, id: String) -> Result<emu::Rom, String> {
    let d = state.direct.get(&id).ok_or("Téléchargement introuvable")?;
    let path = d.final_path();
    let rom = tokio::task::spawn_blocking(move || emu::unpack(&path)).await.map_err(|e| e.to_string())??;
    state.direct.set_rom(&id, &rom.display().to_string());
    Ok(emu::rom_info(&rom))
}

#[tauri::command]
fn emu_find_roms(path: String) -> Vec<emu::Rom> {
    emu::find_roms(std::path::Path::new(&path))
}

#[tauri::command]
async fn emu_install(app: tauri::AppHandle, id: String) -> Result<emu::EmuInfo, String> {
    let m = emu::get(&id).ok_or("Émulateur inconnu")?;
    let a = app.clone();
    emu::install(m, move |pct, msg| {
        let _ = a.emit("emu-progress", serde_json::json!({ "id": m.id, "pct": pct, "msg": msg }));
    })
    .await?;
    Ok(emu::info(m, &emu::installed()))
}

#[tauri::command]
fn emu_play(path: String, id: String) -> Result<(), String> {
    let map = emu::installed();
    let inst = map.get(&id).ok_or("Cet émulateur n'est pas installé")?;
    emu::play(std::path::Path::new(&path), std::path::Path::new(&inst.exe))
}

#[tauri::command]
fn emu_set_exe(id: String, exe: String) -> Result<emu::EmuInfo, String> {
    let m = emu::get(&id).ok_or("Émulateur inconnu")?;
    if !exe.to_lowercase().ends_with(".exe") || !std::path::Path::new(&exe).is_file() {
        return Err("Choisis le fichier .exe de l'émulateur".into());
    }
    emu::set_exe(m.id, &exe, "perso")?;
    Ok(emu::info(m, &emu::installed()))
}

#[tauri::command]
fn emu_remove(id: String) -> Result<(), String> {
    emu::forget(&id)
}

#[tauri::command]
fn emu_take_pending() -> Vec<String> {
    emu::take_pending()
}

/// Ajoute les .torrent / liens magnet reçus au lancement (double-clic sur un fichier, clic sur un lien).
/// Si qBittorrent démarre encore, on attend qu'il soit prêt (40 s max).
fn open_from_args(app: tauri::AppHandle, args: Vec<String>) {
    // ROM double-cliquée : lancée tout de suite si son émulateur est installé, sinon fenêtre « Jouer »
    let roms = emu::rom_args(&args);
    if !roms.is_empty() && emu::open_or_queue(roms) {
        show_main(&app);
        let _ = app.emit("rom-open", ());
    }
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
    let direct = direct::Direct::new();
    direct.set_folders(cfg.folders.clone(), cfg.split_anime);
    let state = AppState {
        qbit: Qbit::new(&cfg),
        direct,
        watched: watch::Watched::new(),
        cfg: Mutex::new(cfg),
        art: Artwork::new(),
        schedules: schedule::Schedules::new(),
        seen: Mutex::new(None),
        seq_done: Mutex::new(HashSet::new()),
        boosted: Mutex::new(HashMap::new()),
        held: Mutex::new(load_held()),
        file_counts: Mutex::new(HashMap::new()),
        content_kind: Mutex::new(HashMap::new()),
        last_progress: Mutex::new(HashMap::new()),
    };
    let app = tauri::Builder::default()
        // Aperçu vidéo de la timeline « Extrait », relayé avec les en-têtes exigés par YouTube
        .register_asynchronous_uri_scheme_protocol("ofapv", |_ctx, request, responder| {
            tauri::async_runtime::spawn(async move { responder.respond(ytdl::proxy(request).await) });
        })
        // Relancer OneForAll alors qu'il tourne déjà → on rouvre simplement la fenêtre
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // Jeu lancé directement dans son émulateur : inutile d'afficher OneForAll
            if emu::rom_args(&args).is_empty() {
                show_main(app);
            }
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
            app.state::<AppState>().direct.start();
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
            emu_list,
            emu_detect,
            emu_find_roms,
            emu_install,
            emu_play,
            emu_set_exe,
            emu_remove,
            emu_take_pending,
            rom_versions,
            rom_download,
            trends_catalog,
            site_popular,
            dl_extract,
            dl_set_kind,
            get_torrents,
            torrent_action,
            get_files,
            get_sources,
            set_file_priority,
            get_categories,
            set_category,
            trash_torrent_files,
            set_location,
            organize_shows,
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
            art_choices,
            set_artwork,
            get_schedule,
            test_notification,
            open_default_apps,
            qbit_status,
            setup_qbit,
            open_url,
            set_custom_icon,
            get_custom_icon,
            get_downloads,
            nxbrew_links,
            get_watch,
            play_files,
            overlay_action,
            overlay_data,
            set_watched,
            add_downloads,
            media_info,
            compress_info,
            compress_video,
            clip_open,
            clip_proxy,
            clip_wave,
            media_wave,
            clip_asset,
            clip_export,
            download_action,
            move_downloads,
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
    fn show_folders() {
        use super::{same_dir, show_folder_name, show_target};
        assert_eq!(show_folder_name(&["JoJos Bizarre Adventure", "JoJo's Bizarre Adventure"]), "JoJo's Bizarre Adventure");
        let f = "JoJo's Bizarre Adventure";
        assert_eq!(show_target(r"D:\Series", f), r"D:\Series\JoJo's Bizarre Adventure");
        // Déjà dans le dossier de la série : on ne crée pas « …\JoJo's…\JoJo's… »
        assert!(same_dir(&show_target(r"D:\Series\JoJo's Bizarre Adventure", f), r"D:\Series\JoJo's Bizarre Adventure"));
    }

    #[test]
    fn kind_from_content() {
        use super::{content_info, Kind, TorrentFile};
        let f = |name: &str| TorrentFile { name: name.into(), size: 400 * 1024 * 1024, ..Default::default() };
        let eps = vec![f("Ao no Hako/01. Chinatsu Senpai.mkv"), f("Ao no Hako/02. You Have to Go to Nationals.mkv"), f("Ao no Hako/03. Chii.mkv")];
        assert_eq!(content_info(&eps).fallback, Some(Kind::Series));
        assert_eq!(content_info(&[f("Inception/Inception.mkv")]).fallback, Some(Kind::Film));
        let films = vec![f("Pack/Alien.mkv"), f("Pack/Aliens.mkv"), f("Pack/Prometheus.mkv")];
        assert_eq!(content_info(&films).fallback, None);
    }

    #[test]
    fn natural_sort() {
        assert_eq!(nat_cmp("Ep 2.mkv", "Ep 10.mkv"), Ordering::Less);
        assert_eq!(nat_cmp("S01E09", "S01E10"), Ordering::Less);
        assert_eq!(nat_cmp("b", "a"), Ordering::Greater);
    }
}