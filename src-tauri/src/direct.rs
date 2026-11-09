//! Téléchargements directs (liens http/https vers un fichier), façon gestionnaire de téléchargements :
//! plusieurs connexions par fichier, reprise après coupure réseau ou redémarrage, file d'attente sauvegardée.
//!
//! Chaque téléchargement apparaît dans la liste principale comme un « torrent » d'identifiant `dl-…`.

use crate::qbit::RawTorrent;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

/// Téléchargements actifs en même temps (les autres attendent leur tour)
const MAX_ACTIVE: usize = 3;
/// Connexions par fichier au maximum, et taille minimale d'un morceau
const MAX_CONN: i64 = 6;
const MIN_SEG: i64 = 8 * 1024 * 1024;
/// Identité honnête : se faire passer pour Chrome sans en être un fait bloquer par certains serveurs
const UA: &str = concat!("OneForAll/", env!("CARGO_PKG_VERSION"));

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum St {
    #[default]
    Queued,
    Running,
    Paused,
    Done,
    Error,
}

/// Morceau du fichier : [start, end[ ; end = -1 tant que la taille est inconnue
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Seg {
    pub start: i64,
    pub end: i64,
    pub done: i64,
}

impl Seg {
    fn complete(&self) -> bool {
        self.end >= 0 && self.start + self.done >= self.end
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Dl {
    pub id: String,
    pub url: String,
    /// Nom final du fichier (vide tant que le serveur n'a pas répondu)
    pub name: String,
    pub dir: String,
    /// -1 = inconnue
    pub size: i64,
    pub resumable: bool,
    pub segs: Vec<Seg>,
    pub st: St,
    pub error: String,
    pub added_on: i64,
    pub completion_on: i64,
    #[serde(skip)]
    pub speed: i64,
    /// Connexion perdue, nouvel essai en attente (wifi coupé…)
    #[serde(skip)]
    pub retrying: bool,
    /// Vidéo / musique d'un site (yt-dlp) : format voulu (max, 1080, 720, mp3) ; vide = fichier classique
    pub media: String,
    /// Miniature de la vidéo
    pub thumb: String,
    /// Étape en cours affichée à la place de l'avancement (installation de ffmpeg, conversion…)
    #[serde(skip)]
    pub note: String,
    /// Extrait seulement : « début-fin » en secondes (vide = tout)
    pub section: String,
    /// Définition réellement téléchargée (« 1080p », « 4K »…)
    pub res: String,
    /// Compression : taille visée en octets
    pub target: i64,
    /// Jeu (site de ROM) : son vrai titre et sa console
    pub game: String,
    pub platform: String,
    /// ROM extraite de l'archive (prête à lancer)
    pub rom: String,
    /// Type choisi à la main (« game », « film »…) ; vide = automatique
    pub kind: String,
    /// Fichier terminé déjà analysé (est-ce une ROM ?)
    pub rom_checked: bool,
}

impl Dl {
    pub fn done(&self) -> i64 {
        self.segs.iter().map(|s| s.done).sum()
    }
    /// Fichier en cours d'écriture d'une compression
    fn compress_tmp(&self) -> PathBuf {
        Path::new(&self.dir).join(format!("{}.part.mp4", self.name))
    }
    pub fn section(&self) -> Option<(u32, u32)> {
        let (a, b) = self.section.split_once('-')?;
        let (a, b) = (a.parse().ok()?, b.parse().ok()?);
        (b > a).then_some((a, b))
    }
    fn display_name(&self) -> String {
        if self.name.is_empty() && !self.media.is_empty() {
            // Le titre n'arrive qu'après l'analyse : « watch » (fin du lien YouTube) ne veut rien dire
            if crate::ytdl::is_spotify(&self.url) { "Morceau Spotify…".into() } else { format!("Vidéo {}…", crate::ytdl::site(&self.url)) }
        } else if self.name.is_empty() {
            file_name_from_url(&self.url).unwrap_or_else(|| self.url.clone())
        } else {
            self.name.clone()
        }
    }
    pub fn final_path(&self) -> PathBuf {
        Path::new(&self.dir).join(&self.name)
    }
    fn part_path(&self) -> PathBuf {
        Path::new(&self.dir).join(format!("{}.part", self.name))
    }
    fn progress(&self) -> f64 {
        match self.st {
            St::Done => 1.0,
            _ if self.size > 0 => (self.done() as f64 / self.size as f64).min(0.998),
            _ => 0.0,
        }
    }

    /// Le téléchargement vu comme un torrent qBittorrent (pour réutiliser détection du type, affiches, groupes…)
    pub fn raw(&self) -> RawTorrent {
        let state = match self.st {
            St::Queued => "queuedDL",
            St::Running if self.retrying => "stalledDL",
            St::Running => "downloading",
            St::Paused => "pausedDL",
            St::Done => "stoppedUP",
            St::Error => "error",
        };
        let size = self.size.max(self.done());
        let remaining = (size - self.done()).max(0);
        let content = if self.name.is_empty() {
            self.dir.clone()
        } else if self.st == St::Done {
            self.final_path().display().to_string()
        } else {
            self.part_path().display().to_string()
        };
        RawTorrent {
            hash: self.id.clone(),
            name: self.display_name(),
            progress: self.progress(),
            state: state.into(),
            size,
            total_size: size,
            completed: self.done(),
            dlspeed: if self.st == St::Running { self.speed } else { 0 },
            eta: if self.st == St::Running && self.speed > 0 && self.size > 0 { remaining / self.speed } else { 8_640_000 },
            added_on: self.added_on,
            completion_on: self.completion_on,
            save_path: self.dir.clone(),
            content_path: content,
            has_metadata: true,
            ..Default::default()
        }
    }
}

pub fn is_direct(hash: &str) -> bool {
    hash.starts_with("dl-")
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn store_path() -> PathBuf {
    crate::config::data_dir().join("downloads.json")
}

fn downloads_dir() -> String {
    let home = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into());
    Path::new(&home).join("Downloads").display().to_string()
}

/// Caractères interdits dans un nom de fichier Windows
fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    s.trim().trim_end_matches('.').to_string()
}

fn file_name_from_url(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let last = path.trim_end_matches('/').rsplit('/').next()?;
    let n = sanitize(&crate::search::pct_decode(last));
    (!n.is_empty() && !n.contains("://") && n.len() < 240).then_some(n)
}

static RE_CD_STAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)filename\*\s*=\s*(?:UTF-8|ISO-8859-1)?'[^']*'([^;]+)").unwrap());
static RE_CD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)filename\s*=\s*"?([^";]+)"?"#).unwrap());
static RE_RANGE_TOTAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/(\d+)\s*$").unwrap());

fn name_from_disposition(cd: &str) -> Option<String> {
    let raw = RE_CD_STAR.captures(cd).map(|c| crate::search::pct_decode(c[1].trim())).or_else(|| RE_CD.captures(cd).map(|c| c[1].trim().to_string()))?;
    let n = sanitize(raw.rsplit(['/', '\\']).next().unwrap_or(&raw));
    (!n.is_empty()).then_some(n)
}

/// Premier nom libre : « fichier.zip », « fichier (1).zip »…
fn unique_name(dir: &Path, name: &str) -> String {
    let free = |n: &str| !dir.join(n).exists() && !dir.join(format!("{n}.part")).exists();
    if free(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (1..1000).map(|i| format!("{stem} ({i}){ext}")).find(|n| free(n)).unwrap_or_else(|| format!("{stem} ({}){ext}", now()))
}

/// Déplace un fichier, même vers un autre disque
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    std::fs::remove_file(from)
}

enum Fail {
    /// Réessayer plus tard (réseau coupé, serveur surchargé…)
    Retry(String),
    /// Inutile d'insister (lien mort, page web…)
    Fatal(String),
    /// Le serveur ne sait plus reprendre en cours de fichier : on repart de zéro, en une seule connexion
    Restart,
}

struct Inner {
    list: Mutex<Vec<Dl>>,
    /// id → drapeau d'arrêt de la tâche en cours
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
    folders: Mutex<(BTreeMap<String, String>, bool)>,
    dirty: AtomicBool,
    seq: AtomicU64,
    client: reqwest::Client,
    /// false dans les tests : ne touche pas au vrai downloads.json
    persist: bool,
}

#[derive(Clone)]
pub struct Direct(Arc<Inner>);

impl Direct {
    pub fn new() -> Self {
        let mut list: Vec<Dl> = std::fs::read_to_string(store_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        // Ce qui tournait à la fermeture reprend tout seul
        for d in list.iter_mut().filter(|d| d.st == St::Running) {
            d.st = St::Queued;
        }
        let client = reqwest::Client::builder()
            .user_agent(UA)
            .connect_timeout(Duration::from_secs(20))
            .build()
            .expect("client http");
        Direct(Arc::new(Inner {
            list: Mutex::new(list),
            running: Mutex::new(HashMap::new()),
            folders: Mutex::new((BTreeMap::new(), true)),
            dirty: AtomicBool::new(false),
            seq: AtomicU64::new(0),
            client,
            persist: true,
        }))
    }

    /// Dossiers par type (Réglages), pour ranger chaque fichier selon ce qu'il contient
    pub fn set_folders(&self, folders: BTreeMap<String, String>, split_anime: bool) {
        *self.0.folders.lock().unwrap() = (folders, split_anime);
    }

    /// Dossier de l'onglet DL (Réglages → Dossiers), sinon Téléchargements
    fn dl_dir(&self) -> String {
        self.0.folders.lock().unwrap().0.get("dl").filter(|f| !f.trim().is_empty()).cloned().unwrap_or_else(downloads_dir)
    }

    /// Compresse une vidéo vers `target` octets : nouvelle ligne « … (compressé).mp4 » à côté de l'originale
    pub fn add_compress(&self, src: &str, target: i64) -> String {
        let p = Path::new(src);
        let dir = p.parent().map(|d| d.display().to_string()).unwrap_or_default();
        let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "video".into());
        let mb = (target as f64 / 1_048_576.0).round() as i64;
        let name = unique_name(Path::new(&dir), &format!("{stem} ({mb} Mo).mp4"));
        let id = self.add_media(src, "compress");
        self.update(&id, |d| {
            d.dir = dir;
            d.name = name;
            d.target = target;
            d.size = target;
        });
        self.save();
        id
    }

    /// ffmpeg : réencode en H.264 au débit qui donne la taille visée. Pause = on arrête, la reprise recommence.
    async fn run_compress(&self, id: &str, stop: &Arc<AtomicBool>) -> Result<(), Fail> {
        let d = self.get(id).ok_or(Fail::Fatal("Retiré".into()))?;
        let me = self.clone();
        let idc = id.to_string();
        let note = move |s: String| {
            me.update(&idc, |d| d.note = s);
        };
        let ff = crate::ytdl::ffmpeg(&self.0.client, &note).await.map_err(Fail::Retry)?;
        let src = PathBuf::from(&d.url);
        if !src.is_file() {
            return Err(Fail::Fatal("La vidéo d'origine n'existe plus.".into()));
        }
        let probe = crate::ytdl::probe(&ff, &src).map_err(Fail::Fatal)?;
        let tmp = d.compress_tmp();
        let args = crate::ytdl::compress_args(&src, &tmp, &probe, d.target);
        let (mut child, rx) = crate::ytdl::spawn_ff(&ff, &args).map_err(|e| Fail::Fatal(format!("Lancement de ffmpeg impossible : {e}")))?;
        self.update(id, |x| x.note = "Compression…".into());
        let mut errors = Vec::new();
        let mut exited = None;
        loop {
            if stop.load(Ordering::Relaxed) {
                crate::ytdl::kill(&mut child);
                let _ = std::fs::remove_file(&tmp);
                self.update(id, |x| {
                    x.note.clear();
                    x.segs.clear();
                });
                return Ok(());
            }
            let mut got = false;
            while let Ok(line) = rx.try_recv() {
                got = true;
                if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                    let frac = (us / 1e6 / probe.duration.max(1.0)).clamp(0.0, 1.0);
                    let size = d.target;
                    self.update(id, |x| {
                        x.segs = vec![Seg { start: 0, end: size, done: (size as f64 * frac) as i64 }];
                        x.note = format!("Compression… {} %", (frac * 100.0) as i64);
                    });
                } else if !line.contains('=') && !line.trim().is_empty() {
                    errors.push(line);
                }
            }
            if exited.is_some() && !got {
                break;
            }
            if exited.is_none() {
                exited = child.try_wait().ok().flatten();
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        if !exited.is_some_and(|s| s.success()) || !tmp.is_file() {
            let _ = std::fs::remove_file(&tmp);
            let msg = errors.last().cloned().unwrap_or_else(|| "ffmpeg s'est arrêté".into());
            return Err(Fail::Fatal(format!("Compression impossible : {}", msg.chars().take(200).collect::<String>())));
        }
        let fin = Path::new(&d.dir).join(&d.name);
        std::fs::rename(&tmp, &fin).map_err(|e| Fail::Fatal(e.to_string()))?;
        let len = std::fs::metadata(&fin).map(|m| m.len() as i64).unwrap_or(0);
        self.update(id, |x| {
            x.size = len;
            x.segs = vec![Seg { start: 0, end: len, done: len }];
            x.st = St::Done;
            x.completion_on = now();
            x.note.clear();
            x.error.clear();
        });
        self.save();
        Ok(())
    }

    /// Export d'extraits (studio) : `spec` = clip::Spec en JSON, gardé dans `url`
    pub fn add_clip(&self, spec: &crate::clip::Spec, dir: &str, name: &str) -> String {
        let _ = std::fs::create_dir_all(dir);
        let name = unique_name(Path::new(dir), name);
        let id = self.add_media(&serde_json::to_string(spec).unwrap_or_default(), "clip");
        self.update(&id, |d| {
            d.dir = dir.to_string();
            d.name = name;
        });
        self.save();
        id
    }

    async fn run_clip(&self, id: &str, stop: &Arc<AtomicBool>) -> Result<(), Fail> {
        let d = self.get(id).ok_or(Fail::Fatal("Retiré".into()))?;
        let spec: crate::clip::Spec = serde_json::from_str(&d.url).map_err(|_| Fail::Fatal("Extrait illisible".into()))?;
        let me = self.clone();
        let idc = id.to_string();
        let note = move |s: String| {
            me.update(&idc, |d| d.note = s);
        };
        let ff = crate::ytdl::ffmpeg(&self.0.client, &note).await.map_err(Fail::Retry)?;
        let src = crate::clip::open(&ff, &spec.path).map_err(Fail::Fatal)?;
        // Sous-titres extraits avant l'export (export repris après un redémarrage, sans passer par le studio)
        let s2 = src.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || crate::clip::prepare(&s2)).await;
        let tmp = crate::ytdl::tmp_dir(id);
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| Fail::Fatal(e.to_string()))?;
        let total: f64 = spec.segs.iter().map(|s| (s.end - s.start).max(0.1)).sum();
        let mut done_secs = 0.0;
        let mut parts = Vec::new();
        for (i, seg) in spec.segs.iter().enumerate() {
            let out = tmp.join(format!("p{i}.mp4"));
            let args = crate::clip::export_full(&src, &spec, seg, &out, &tmp);
            let mut cmd = crate::clip::ffmpeg_in(&src);
            cmd.args(&args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
            let mut child = cmd.spawn().map_err(|e| Fail::Fatal(format!("Lancement de ffmpeg impossible : {e}")))?;
            let rx = crate::ytdl::pipe_lines(&mut child);
            let label = if spec.segs.len() > 1 { format!("Passage {}/{}", i + 1, spec.segs.len()) } else { "Export".into() };
            let mut errors = Vec::new();
            let mut exited = None;
            loop {
                if stop.load(Ordering::Relaxed) {
                    crate::ytdl::kill(&mut child);
                    let _ = std::fs::remove_dir_all(&tmp);
                    self.update(id, |x| {
                        x.note.clear();
                        x.segs.clear();
                    });
                    return Ok(());
                }
                let mut got = false;
                while let Ok(line) = rx.try_recv() {
                    got = true;
                    if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                        let frac = ((done_secs + us / 1e6) / total).clamp(0.0, 1.0);
                        let l = label.clone();
                        self.update(id, |x| {
                            x.size = 1000;
                            x.segs = vec![Seg { start: 0, end: 1000, done: (frac * 1000.0) as i64 }];
                            x.note = format!("{l}… {} %", (frac * 100.0) as i64);
                        });
                    } else if !line.contains('=') && !line.trim().is_empty() {
                        errors.push(line);
                    }
                }
                if exited.is_some() && !got {
                    break;
                }
                if exited.is_none() {
                    exited = child.try_wait().ok().flatten();
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            if !exited.is_some_and(|s| s.success()) || !out.is_file() {
                let _ = std::fs::remove_dir_all(&tmp);
                let msg = errors.last().cloned().unwrap_or_else(|| "ffmpeg s'est arrêté".into());
                return Err(Fail::Fatal(format!("Export impossible : {}", msg.chars().take(200).collect::<String>())));
            }
            done_secs += (seg.end - seg.start).max(0.1);
            parts.push(out);
        }
        let fin = Path::new(&d.dir).join(&d.name);
        if parts.len() == 1 {
            move_file(&parts[0], &fin).map_err(|e| Fail::Fatal(e.to_string()))?;
        } else {
            // Montage : les passages ont le même format, on les colle sans réencoder
            self.update(id, |x| x.note = "Assemblage…".into());
            let list = tmp.join("list.txt");
            let body: String = parts.iter().map(|p| format!("file '{}'\n", p.display().to_string().replace('\'', "'\\''"))).collect();
            std::fs::write(&list, body).map_err(|e| Fail::Fatal(e.to_string()))?;
            let st = crate::clip::ffmpeg_in(&src)
                .args(["-v", "error", "-y", "-nostdin", "-f", "concat", "-safe", "0", "-i"])
                .arg(&list)
                .args(["-c", "copy", "-movflags", "+faststart"])
                .arg(&fin)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            if !st.is_ok_and(|s| s.success()) {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(Fail::Fatal("Assemblage des passages impossible".into()));
            }
        }
        let _ = std::fs::remove_dir_all(&tmp);
        let len = std::fs::metadata(&fin).map(|m| m.len() as i64).unwrap_or(0);
        self.update(id, |x| {
            x.size = len;
            x.segs = vec![Seg { start: 0, end: len, done: len }];
            x.st = St::Done;
            x.completion_on = now();
            x.note.clear();
            x.error.clear();
        });
        self.save();
        Ok(())
    }

    pub fn set_section(&self, id: &str, section: &str) {
        self.update(id, |d| d.section = section.to_string());
        self.save();
    }

    pub fn list(&self) -> Vec<Dl> {
        self.0.list.lock().unwrap().clone()
    }

    pub fn get(&self, id: &str) -> Option<Dl> {
        self.0.list.lock().unwrap().iter().find(|d| d.id == id).cloned()
    }

    /// Jeu d'un site de ROM : titre, console et jaquette connus d'avance
    pub fn add_game(&self, url: &str, game: &str, platform: &str, thumb: &str) -> String {
        let id = self.add(url);
        self.update(&id, |d| {
            d.game = game.trim().to_string();
            d.platform = platform.trim().to_string();
            d.thumb = thumb.trim().to_string();
        });
        self.save();
        id
    }

    pub fn set_kind(&self, id: &str, kind: &str) {
        self.update(id, |d| d.kind = kind.to_string());
        self.save();
    }

    /// Fichiers terminés pas encore analysés (ROM ?) : marqués tout de suite pour ne les analyser qu'une fois
    pub fn take_unchecked(&self, is_candidate: impl Fn(&Dl) -> bool) -> Vec<Dl> {
        let mut out = Vec::new();
        for d in self.0.list.lock().unwrap().iter_mut() {
            if d.st == St::Done && !d.rom_checked && d.game.is_empty() && d.media.is_empty() {
                d.rom_checked = true;
                if is_candidate(d) {
                    out.push(d.clone());
                }
            }
        }
        if !out.is_empty() {
            self.0.dirty.store(true, Ordering::Relaxed);
        }
        out
    }

    /// Jeu reconnu après coup (ROM téléchargée sans passer par un site de ROM)
    pub fn set_game(&self, id: &str, game: &str, platform: &str) {
        self.update(id, |d| {
            d.game = game.to_string();
            d.platform = platform.to_string();
        });
        self.save();
    }

    /// ROM extraite de l'archive téléchargée
    pub fn set_rom(&self, id: &str, rom: &str) {
        self.update(id, |d| d.rom = rom.to_string());
        self.save();
    }

    /// Archive extraite puis supprimée : le téléchargement devient le dossier extrait
    pub fn set_extracted(&self, id: &str, rom: &str, folder: &str) {
        self.update(id, |d| {
            d.rom = rom.to_string();
            d.name = folder.to_string();
        });
        self.save();
    }

    pub fn add(&self, url: &str) -> String {
        self.add_media(url, "")
    }

    /// `media` : format yt-dlp (max, 1080, 720, mp3), ou vide pour un fichier classique
    pub fn add_media(&self, url: &str, media: &str) -> String {
        let n = self.0.seq.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let id = format!("dl-{:x}{n:x}", nanos);
        let dl = Dl { id: id.clone(), url: url.trim().to_string(), size: -1, st: St::Queued, added_on: now(), media: media.to_string(), ..Default::default() };
        self.0.list.lock().unwrap().push(dl);
        self.save();
        id
    }

    fn save(&self) {
        if !self.0.persist {
            return;
        }
        let json = serde_json::to_string_pretty(&*self.0.list.lock().unwrap()).unwrap_or_default();
        let _ = std::fs::create_dir_all(crate::config::data_dir());
        let tmp = store_path().with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, store_path());
        }
        self.0.dirty.store(false, Ordering::Relaxed);
    }

    fn update<R>(&self, id: &str, f: impl FnOnce(&mut Dl) -> R) -> Option<R> {
        let mut list = self.0.list.lock().unwrap();
        let r = list.iter_mut().find(|d| d.id == id).map(f);
        self.0.dirty.store(true, Ordering::Relaxed);
        r
    }

    fn ids_for(&self, ids: &[String]) -> Vec<String> {
        let list = self.0.list.lock().unwrap();
        if ids.iter().any(|i| i == "all") {
            list.iter().map(|d| d.id.clone()).collect()
        } else {
            ids.iter().filter(|i| is_direct(i)).cloned().collect()
        }
    }

    fn stop(&self, id: &str) {
        if let Some(flag) = self.0.running.lock().unwrap().get(id) {
            flag.store(true, Ordering::Relaxed);
        }
    }

    async fn wait_stopped(&self, id: &str) {
        for _ in 0..100 {
            if !self.0.running.lock().unwrap().contains_key(id) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// pause, resume, recheck (= réessayer), delete, delete_files
    pub async fn action(&self, ids: &[String], action: &str) -> Result<(), String> {
        let ids = self.ids_for(ids);
        match action {
            "pause" => {
                for id in &ids {
                    self.update(id, |d| {
                        if matches!(d.st, St::Queued | St::Running) {
                            d.st = St::Paused;
                        }
                    });
                    self.stop(id);
                }
            }
            "resume" | "recheck" => {
                for id in &ids {
                    self.update(id, |d| {
                        if matches!(d.st, St::Paused | St::Error) {
                            d.st = St::Queued;
                            d.error.clear();
                            d.note.clear();
                        }
                    });
                }
            }
            "delete" | "delete_files" => {
                for id in &ids {
                    self.stop(id);
                    self.wait_stopped(id).await;
                    let removed = {
                        let mut list = self.0.list.lock().unwrap();
                        let i = list.iter().position(|d| &d.id == id);
                        i.map(|i| list.remove(i))
                    };
                    let _ = std::fs::remove_dir_all(crate::ytdl::tmp_dir(id));
                    if let Some(d) = removed.filter(|d| !d.name.is_empty()) {
                        // Un fichier inachevé ne sert à rien : il part toujours. Le fichier terminé, seulement si demandé.
                        let _ = std::fs::remove_file(d.part_path());
                        let _ = std::fs::remove_file(d.compress_tmp());
                        if action == "delete_files" && d.final_path().exists() {
                            trash::delete(d.final_path()).map_err(|e| format!("Suppression impossible : {e}"))?;
                        }
                    }
                }
            }
            "toggle_seq" | "toggle_flpp" => {}
            other => return Err(format!("Action inconnue : {other}")),
        }
        self.save();
        Ok(())
    }

    /// Déplace des téléchargements (terminés ou non) dans un autre dossier.
    pub async fn set_dir(&self, ids: &[String], dir: &str) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        for id in self.ids_for(ids) {
            let was = self.get(&id).map(|d| d.st);
            self.stop(&id);
            self.wait_stopped(&id).await;
            let Some(d) = self.get(&id) else { continue };
            if !d.name.is_empty() {
                let target = Path::new(dir);
                for (from, to) in [(d.final_path(), target.join(&d.name)), (d.part_path(), target.join(format!("{}.part", d.name)))] {
                    if from.is_file() && from != to {
                        move_file(&from, &to).map_err(|e| format!("Déplacement impossible : {e}"))?;
                    }
                }
            }
            self.update(&id, |d| {
                d.dir = dir.to_string();
                if was == Some(St::Running) {
                    d.st = St::Queued;
                }
            });
        }
        self.save();
        Ok(())
    }

    /// Boucle de fond : vitesses, file d'attente, sauvegarde.
    pub fn start(&self) {
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut last: HashMap<String, i64> = HashMap::new();
            let mut tick = 0u64;
            loop {
                {
                    let mut list = me.0.list.lock().unwrap();
                    for d in list.iter_mut() {
                        let done = d.done();
                        let prev = last.insert(d.id.clone(), done).unwrap_or(done);
                        let inst = (done - prev).max(0);
                        d.speed = if d.st == St::Running { (d.speed * 2 + inst * 3) / 5 } else { 0 };
                    }
                }
                // File d'attente : on lance les suivants
                let to_start: Vec<String> = {
                    let running = me.0.running.lock().unwrap().len();
                    let list = me.0.list.lock().unwrap();
                    list.iter().filter(|d| d.st == St::Queued).take(MAX_ACTIVE.saturating_sub(running)).map(|d| d.id.clone()).collect()
                };
                for id in to_start {
                    let flag = Arc::new(AtomicBool::new(false));
                    me.0.running.lock().unwrap().insert(id.clone(), flag.clone());
                    me.update(&id, |d| d.st = St::Running);
                    let me2 = me.clone();
                    tauri::async_runtime::spawn(async move {
                        me2.run(&id, flag).await;
                        me2.0.running.lock().unwrap().remove(&id);
                        me2.save();
                    });
                }
                tick += 1;
                if tick % 3 == 0 && me.0.dirty.load(Ordering::Relaxed) {
                    me.save();
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
    }

    async fn run(&self, id: &str, stop: Arc<AtomicBool>) {
        let mut attempt = 0u32;
        loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let res = self.run_once(id, &stop).await;
            if stop.load(Ordering::Relaxed) {
                return;
            }
            match res {
                Ok(()) => return self.finish(id),
                Err(Fail::Fatal(msg)) => {
                    self.update(id, |d| {
                        d.st = St::Error;
                        d.error = msg;
                        d.retrying = false;
                    });
                    return;
                }
                Err(Fail::Restart) => {
                    self.update(id, |d| {
                        d.resumable = false;
                        d.segs = vec![Seg { start: 0, end: d.size, done: 0 }];
                    });
                }
                Err(Fail::Retry(msg)) if attempt >= 5 && self.get(id).is_some_and(|d| !d.media.is_empty()) => {
                    self.update(id, |d| {
                        d.st = St::Error;
                        d.error = msg;
                        d.retrying = false;
                        d.note.clear();
                    });
                    return;
                }
                Err(Fail::Retry(msg)) => {
                    attempt += 1;
                    self.update(id, |d| {
                        d.retrying = true;
                        d.error = msg;
                    });
                    let wait = [2u64, 4, 8, 15, 30][(attempt as usize - 1).min(4)];
                    for _ in 0..wait * 2 {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
            }
        }
    }

    fn finish(&self, id: &str) {
        let Some(d) = self.get(id) else { return };
        if d.st == St::Done {
            return;
        }
        let dir = Path::new(&d.dir);
        // Le nom final a pu être pris entre-temps
        let name = if d.final_path().exists() { unique_name(dir, &d.name) } else { d.name.clone() };
        match std::fs::rename(d.part_path(), dir.join(&name)) {
            Ok(()) => {
                self.update(id, |d| {
                    d.name = name;
                    d.size = d.done();
                    d.st = St::Done;
                    d.completion_on = now();
                    d.retrying = false;
                    d.error.clear();
                });
            }
            Err(e) => {
                self.update(id, |d| {
                    d.st = St::Error;
                    d.error = format!("Fichier téléchargé, mais impossible de le renommer : {e}");
                });
            }
        }
        self.save();
    }

    /// Première réponse du serveur : nom, taille, reprise possible ? Puis découpage en morceaux.
    async fn probe(&self, id: &str, url: &str) -> Result<(), Fail> {
        let res = self.0.client.get(url).header("Range", "bytes=0-").send().await.map_err(|e| Fail::Retry(net_err(&e)))?;
        let status = res.status();
        check_status(status)?;
        let h = res.headers();
        let text = |k: reqwest::header::HeaderName| h.get(k).map(|v| String::from_utf8_lossy(v.as_bytes()).to_string()).unwrap_or_default();
        let ctype = text(reqwest::header::CONTENT_TYPE).to_lowercase();
        let disposition = text(reqwest::header::CONTENT_DISPOSITION);
        let name = name_from_disposition(&disposition)
            .or_else(|| file_name_from_url(res.url().as_str()))
            .or_else(|| file_name_from_url(url))
            .unwrap_or_else(|| "telechargement".into());
        let lname = name.to_lowercase();
        if ctype.starts_with("text/html") && disposition.is_empty() && !lname.ends_with(".html") && !lname.ends_with(".htm") {
            // Page web : peut-être une vidéo d'un site que yt-dlp connaît
            self.update(id, |d| d.media = "max".into());
            return Ok(());
        }
        let (size, resumable) = if status == reqwest::StatusCode::PARTIAL_CONTENT {
            let total = RE_RANGE_TOTAL.captures(&text(reqwest::header::CONTENT_RANGE)).and_then(|c| c[1].parse().ok()).unwrap_or(-1);
            (total, total > 0)
        } else {
            let len = res.content_length().map(|l| l as i64).unwrap_or(-1);
            (len, len > 0 && text(reqwest::header::ACCEPT_RANGES).contains("bytes"))
        };
        drop(res);

        // Tout ce qui passe par l'onglet DL va dans le même dossier (Réglages → Dossiers → Onglet DL)
        let dir = {
            let existing = self.get(id).map(|d| d.dir).unwrap_or_default();
            if existing.is_empty() { self.dl_dir() } else { existing }
        };
        std::fs::create_dir_all(&dir).map_err(|e| Fail::Fatal(format!("Dossier « {dir} » inaccessible : {e}")))?;
        let name = unique_name(Path::new(&dir), &name);

        let n = if resumable { (size / MIN_SEG).clamp(1, MAX_CONN) } else { 1 };
        let segs: Vec<Seg> = if n > 1 {
            let step = size / n;
            (0..n).map(|i| Seg { start: i * step, end: if i == n - 1 { size } else { (i + 1) * step }, done: 0 }).collect()
        } else {
            vec![Seg { start: 0, end: size, done: 0 }]
        };
        let part = Path::new(&dir).join(format!("{name}.part"));
        let f = std::fs::File::create(&part).map_err(|e| Fail::Fatal(format!("Création du fichier impossible : {e}")))?;
        if size > 0 {
            // Réserve la place d'un coup (et échoue tout de suite si le disque est plein)
            f.set_len(size as u64).map_err(|e| Fail::Fatal(format!("Pas assez de place sur le disque : {e}")))?;
        }
        self.update(id, |d| {
            d.name = name;
            d.dir = dir;
            d.size = size;
            d.resumable = resumable;
            d.segs = segs;
        });
        self.save();
        Ok(())
    }

    async fn run_once(&self, id: &str, stop: &Arc<AtomicBool>) -> Result<(), Fail> {
        let Some(d) = self.get(id) else { return Ok(()) };
        if d.media == "compress" {
            return self.run_compress(id, stop).await;
        }
        if d.media == "clip" {
            return self.run_clip(id, stop).await;
        }
        if !d.media.is_empty() {
            return self.run_media(id, stop).await;
        }
        if d.segs.is_empty() || d.name.is_empty() {
            self.probe(id, &d.url).await?;
            if self.get(id).is_some_and(|d| !d.media.is_empty()) {
                return self.run_media(id, stop).await;
            }
        } else if !d.part_path().is_file() {
            // .part supprimé à la main : on recommence
            self.update(id, |d| {
                d.name.clear();
                d.segs.clear();
            });
            return Err(Fail::Retry("Fichier partiel introuvable, nouveau départ".into()));
        }
        let d = self.get(id).ok_or(Fail::Fatal("Téléchargement retiré".into()))?;
        let tasks: Vec<_> = (0..d.segs.len())
            .filter(|&i| !d.segs[i].complete())
            .map(|i| {
                let me = self.clone();
                let id = id.to_string();
                let stop = stop.clone();
                tauri::async_runtime::spawn(async move { me.segment(&id, i, &stop).await })
            })
            .collect();
        let mut result = Ok(());
        for t in tasks {
            let r = t.await.unwrap_or(Err(Fail::Retry("Tâche interrompue".into())));
            // Le plus grave l'emporte : Fatal > Restart > Retry
            result = match (result, r) {
                (Err(Fail::Fatal(m)), _) | (_, Err(Fail::Fatal(m))) => Err(Fail::Fatal(m)),
                (Err(Fail::Restart), _) | (_, Err(Fail::Restart)) => Err(Fail::Restart),
                (Err(e), _) | (_, Err(e)) => Err(e),
                _ => Ok(()),
            };
        }
        result
    }

    /// Vidéo ou musique d'un site, via yt-dlp. Pause = on arrête yt-dlp ; il reprend ses fichiers partiels au lancement suivant.
    async fn run_media(&self, id: &str, stop: &Arc<AtomicBool>) -> Result<(), Fail> {
        let d = self.get(id).ok_or(Fail::Fatal("Téléchargement retiré".into()))?;
        let me = self.clone();
        let idc = id.to_string();
        let note = move |s: String| {
            me.update(&idc, |d| d.note = s);
        };
        let exe = crate::ytdl::ytdlp(&self.0.client, &note).await.map_err(Fail::Retry)?;
        let ff = crate::ytdl::ffmpeg(&self.0.client, &note).await.map_err(Fail::Retry)?;
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }

        // Spotify : une playlist / un album devient un téléchargement par morceau ; un morceau est cherché sur YouTube
        let mut track: Option<(String, u32)> = None;
        if crate::ytdl::is_spotify(&d.url) {
            note("Lecture de Spotify…".into());
            match crate::ytdl::spotify(&self.0.client, &d.url).await.map_err(Fail::Retry)? {
                crate::ytdl::Spotify::Track(name, secs) => {
                    let n = sanitize(&name);
                    self.update(id, |x| x.name = n.clone());
                    track = Some((n, secs));
                }
                crate::ytdl::Spotify::List(urls) => {
                    for u in &urls {
                        let nid = self.add_media(u, "mp3");
                        self.update(&nid, |x| x.dir = d.dir.clone());
                    }
                    // La playlist elle-même disparaît, remplacée par ses morceaux
                    self.0.list.lock().unwrap().retain(|x| x.id != id);
                    self.save();
                    return Ok(());
                }
            }
        } else if crate::ytdl::is_playlist_url(&d.url) {
            // Playlist : un téléchargement par élément, celui-ci prend le premier
            note("Lecture de la playlist…".into());
            let urls = crate::ytdl::expand(&exe, &d.url).await;
            let Some(first) = urls.first() else {
                return Err(Fail::Fatal("Playlist vide ou introuvable.".into()));
            };
            for u in &urls[1..] {
                let nid = self.add_media(u, &d.media);
                self.update(&nid, |x| x.dir = d.dir.clone());
            }
            self.update(id, |x| x.url = first.clone());
            self.save();
        }
        let d = self.get(id).ok_or(Fail::Fatal("Téléchargement retiré".into()))?;

        let dir = if d.dir.is_empty() { self.dl_dir() } else { d.dir.clone() };
        std::fs::create_dir_all(&dir).map_err(|e| Fail::Fatal(format!("Dossier « {dir} » inaccessible : {e}")))?;
        let tmp = crate::ytdl::tmp_dir(id);
        let _ = std::fs::create_dir_all(&tmp);
        self.update(id, |x| {
            x.dir = dir.clone();
            x.note = if track.is_some() { "Recherche sur YouTube…".into() } else { "Analyse du lien…".into() };
        });

        let search = track.as_ref().map(|(n, _)| format!("ytsearch5:{n}"));
        let job = crate::ytdl::Job {
            url: search.as_deref().unwrap_or(&d.url),
            fmt: if track.is_some() { "mp3" } else { &d.media },
            dir: &dir,
            tmp: &tmp,
            ffmpeg: &ff,
            section: d.section(),
            track: track.as_ref().map(|(n, s)| (n.as_str(), *s)),
        };
        let args = crate::ytdl::args(&job);
        let (mut child, rx) = crate::ytdl::spawn(&exe, &args).map_err(|e| Fail::Fatal(format!("Lancement de yt-dlp impossible : {e}")))?;
        // Image et son se téléchargent l'un après l'autre : on additionne
        let (mut base, mut cur_done, mut cur_total, mut est) = (0i64, 0i64, 0i64, -1i64);
        let mut file: Option<String> = None;
        let mut errors: Vec<String> = Vec::new();
        let mut exited = None;
        loop {
            if stop.load(Ordering::Relaxed) {
                crate::ytdl::kill(&mut child);
                self.update(id, |x| x.note.clear());
                return Ok(());
            }
            let mut got = false;
            while let Ok(line) = rx.try_recv() {
                got = true;
                use crate::ytdl::Line;
                match crate::ytdl::parse(&line) {
                    Line::Title(t) if track.is_none() => {
                        // Un extrait passe par ffmpeg, qui ne donne pas d'avancement : on le dit plutôt que de rester sur « Analyse »
                        let cutting = d.section().is_some();
                        self.update(id, |x| {
                            x.name = sanitize(&t);
                            if cutting {
                                x.note = "Téléchargement de l'extrait…".into();
                            }
                        });
                    }
                    Line::Thumb(t) => {
                        self.update(id, |x| x.thumb = t);
                    }
                    Line::Size(n) if n > 0 => est = n,
                    Line::Height(h) if d.media != "mp3" => {
                        self.update(id, |x| x.res = crate::ytdl::res_label(h));
                    }
                    Line::Progress(done, total) => {
                        if done + 1024 * 1024 < cur_done {
                            base += cur_total.max(cur_done);
                        }
                        cur_done = done;
                        cur_total = total.max(done);
                        let sum = base + cur_done;
                        let size = est.max(base + cur_total);
                        let finishing = total > 0 && done >= total;
                        self.update(id, |x| {
                            x.segs = vec![Seg { start: 0, end: size, done: sum }];
                            x.size = size;
                            x.retrying = false;
                            x.error.clear();
                            x.note = if finishing { "Finalisation…".into() } else { String::new() };
                        });
                    }
                    Line::File(f) => file = Some(f),
                    Line::Error(e) => errors.push(e),
                    _ => {}
                }
            }
            if exited.is_some() && !got {
                break;
            }
            if exited.is_none() {
                exited = child.try_wait().ok().flatten();
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        // 101 : « --max-downloads » atteint (recherche Spotify), c.-à-d. réussi
        let ok = exited.is_some_and(|s| s.success() || s.code() == Some(101));
        let found = file.as_deref().map(PathBuf::from).filter(|p| p.is_file());
        if let (true, Some(path)) = (ok, found) {
            let len = std::fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0);
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let parent = path.parent().map(|p| p.display().to_string()).unwrap_or(dir);
            self.update(id, |x| {
                x.name = name;
                x.dir = parent;
                x.size = len;
                x.segs = vec![Seg { start: 0, end: len, done: len }];
                x.st = St::Done;
                x.completion_on = now();
                x.retrying = false;
                x.error.clear();
                x.note.clear();
            });
            let _ = std::fs::remove_dir_all(&tmp);
            self.save();
            return Ok(());
        }
        self.update(id, |x| x.note.clear());
        let last = errors.last().cloned().unwrap_or_else(|| if ok { "yt-dlp n'a produit aucun fichier".into() } else { "yt-dlp s'est arrêté sans explication".into() });
        let (msg, fatal) = crate::ytdl::explain(&last);
        Err(if fatal { Fail::Fatal(msg) } else { Fail::Retry(msg) })
    }

    /// Télécharge un morceau, en reprenant là où il s'était arrêté.
    async fn segment(&self, id: &str, i: usize, stop: &Arc<AtomicBool>) -> Result<(), Fail> {
        let d = self.get(id).ok_or(Fail::Fatal("Téléchargement retiré".into()))?;
        let seg = d.segs[i].clone();
        let mut pos = seg.start + seg.done;
        let mut req = self.0.client.get(&d.url);
        if d.resumable {
            let range = if seg.end > 0 { format!("bytes={pos}-{}", seg.end - 1) } else { format!("bytes={pos}-") };
            req = req.header("Range", range);
        } else if pos > 0 {
            // Serveur sans reprise : on repart du début
            return Err(Fail::Restart);
        }
        let mut res = req.send().await.map_err(|e| Fail::Retry(net_err(&e)))?;
        check_status(res.status())?;
        if pos > 0 && res.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(Fail::Restart);
        }
        let mut file = std::fs::OpenOptions::new().write(true).open(d.part_path()).map_err(|e| Fail::Fatal(format!("Fichier inaccessible : {e}")))?;
        file.seek(SeekFrom::Start(pos as u64)).map_err(|e| Fail::Fatal(e.to_string()))?;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            let chunk = match tokio::time::timeout(Duration::from_secs(30), res.chunk()).await {
                Err(_) => return Err(Fail::Retry("Plus de réponse du serveur".into())),
                Ok(Err(e)) => return Err(Fail::Retry(net_err(&e))),
                Ok(Ok(c)) => c,
            };
            let Some(mut bytes) = chunk else {
                // Fin du flux : taille inconnue → c'était la fin du fichier ; sinon la connexion a coupé trop tôt
                if seg.end < 0 {
                    self.update(id, |d| {
                        d.segs[i].end = pos;
                        d.size = pos;
                    });
                    return Ok(());
                }
                return if pos >= seg.end { Ok(()) } else { Err(Fail::Retry("Connexion coupée".into())) };
            };
            if seg.end > 0 && pos + bytes.len() as i64 > seg.end {
                bytes.truncate((seg.end - pos) as usize);
            }
            file.write_all(&bytes).map_err(|e| Fail::Fatal(format!("Écriture impossible (disque plein ?) : {e}")))?;
            pos += bytes.len() as i64;
            let n = bytes.len() as i64;
            self.update(id, |d| {
                d.segs[i].done += n;
                if d.retrying {
                    d.retrying = false;
                    d.error.clear();
                }
            });
            if seg.end > 0 && pos >= seg.end {
                return Ok(());
            }
        }
    }
}

fn net_err(e: &reqwest::Error) -> String {
    if e.is_connect() {
        "Pas de connexion — nouvel essai automatique".into()
    } else if e.is_timeout() {
        "Le serveur ne répond pas — nouvel essai automatique".into()
    } else {
        format!("Connexion perdue ({e}) — nouvel essai automatique")
    }
}

fn check_status(s: reqwest::StatusCode) -> Result<(), Fail> {
    match s.as_u16() {
        200..=299 => Ok(()),
        404 | 410 => Err(Fail::Fatal("Lien mort : le fichier n'existe plus sur le serveur.".into())),
        401 | 403 => Err(Fail::Fatal(format!("Accès refusé par le serveur ({s}) : le lien a peut-être expiré."))),
        416 => Err(Fail::Restart),
        _ => Err(Fail::Retry(format!("Le serveur a répondu {s} — nouvel essai automatique"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(name_from_disposition(r#"attachment; filename="Film 2024.mkv""#).as_deref(), Some("Film 2024.mkv"));
        assert_eq!(name_from_disposition("attachment; filename*=UTF-8''Jeu%20%C3%A9t%C3%A9.zip").as_deref(), Some("Jeu été.zip"));
        assert_eq!(file_name_from_url("https://x.org/files/My%20Game.v1.rar?token=1").as_deref(), Some("My Game.v1.rar"));
        assert_eq!(sanitize("a:b?.txt"), "a_b_.txt");
        assert!(is_direct("dl-abc") && !is_direct("abcdef"));
    }

    /// Réseau : cargo test direct_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn direct_live() {
        let dir = std::env::temp_dir().join("oneforall-direct-test");
        let _ = std::fs::remove_dir_all(&dir);
        let d = Direct(Arc::new(Inner {
            list: Mutex::new(Vec::new()),
            running: Mutex::new(HashMap::new()),
            folders: Mutex::new((BTreeMap::new(), true)),
            dirty: AtomicBool::new(false),
            seq: AtomicU64::new(0),
            client: reqwest::Client::builder().user_agent(UA).build().unwrap(),
            persist: false,
        }));
        let url = "https://speed.hetzner.de/100MB.bin";
        let url = std::env::var("DL_URL").unwrap_or(url.into());
        let id = d.add(&url);
        d.update(&id, |x| x.dir = dir.display().to_string());
        tauri::async_runtime::block_on(async {
            // 1er passage interrompu au bout de 3 s, puis reprise
            let stop = Arc::new(AtomicBool::new(false));
            let s2 = stop.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                s2.store(true, Ordering::Relaxed);
            });
            d.run(&id, stop).await;
            let mid = d.get(&id).unwrap();
            println!("après coupure : {} / {} octets, {} morceaux, reprise={}", mid.done(), mid.size, mid.segs.len(), mid.resumable);
            d.run(&id, Arc::new(AtomicBool::new(false))).await;
        });
        let f = d.get(&id).unwrap();
        println!("{:?} {} {} {}", f.st, f.name, f.size, f.error);
        assert_eq!(f.st, St::Done);
        assert_eq!(std::fs::metadata(f.final_path()).unwrap().len() as i64, f.size);
    }
}

#[cfg(test)]
mod media_tests {
    use super::*;

    /// Réseau : cargo test media_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn media_live() {
        let dir = std::env::temp_dir().join("oneforall-media-test");
        let _ = std::fs::remove_dir_all(&dir);
        let d = Direct(Arc::new(Inner {
            list: Mutex::new(Vec::new()),
            running: Mutex::new(HashMap::new()),
            folders: Mutex::new((BTreeMap::new(), true)),
            dirty: AtomicBool::new(false),
            seq: AtomicU64::new(0),
            client: reqwest::Client::builder().user_agent(UA).build().unwrap(),
            persist: false,
        }));
        let url = std::env::var("YT_URL").unwrap_or("https://www.youtube.com/watch?v=jNQXAC9IVRw".into());
        for fmt in ["mp3", "720"] {
            let id = d.add_media(&url, fmt);
            d.update(&id, |x| x.dir = dir.display().to_string());
            tauri::async_runtime::block_on(d.run(&id, Arc::new(AtomicBool::new(false))));
            let f = d.get(&id).unwrap();
            println!("{fmt}: {:?} « {} » {} o, miniature={} err={}", f.st, f.name, f.size, !f.thumb.is_empty(), f.error);
            assert_eq!(f.st, St::Done);
            assert!(f.final_path().is_file());
        }
    }
}

#[cfg(test)]
mod media_tests2 {
    use super::*;

    /// Réseau : cargo test media_extra -- --ignored --nocapture
    #[test]
    #[ignore]
    fn media_extra() {
        let dir = std::env::temp_dir().join("oneforall-media-test2");
        let _ = std::fs::remove_dir_all(&dir);
        let d = Direct(Arc::new(Inner {
            list: Mutex::new(Vec::new()),
            running: Mutex::new(HashMap::new()),
            folders: Mutex::new((BTreeMap::new(), true)),
            dirty: AtomicBool::new(false),
            seq: AtomicU64::new(0),
            client: reqwest::Client::builder().user_agent(UA).build().unwrap(),
            persist: false,
        }));
        let cut = d.add_media("https://www.youtube.com/watch?v=jNQXAC9IVRw", "720");
        d.set_section(&cut, "5-12");
        let sp = d.add_media("https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT", "mp3");
        for id in [cut, sp] {
            d.update(&id, |x| x.dir = dir.display().to_string());
            tauri::async_runtime::block_on(d.run(&id, Arc::new(AtomicBool::new(false))));
            let f = d.get(&id).unwrap();
            println!("{:?} « {} » {} o err={}", f.st, f.name, f.size, f.error);
            assert_eq!(f.st, St::Done);
        }
    }
}

#[cfg(test)]
mod clip_tests {
    use super::*;

    /// cargo test clip_job_live -- --ignored --nocapture  (CLIP=vidéo)
    #[test]
    #[ignore]
    fn clip_job_live() {
        let d = Direct(Arc::new(Inner {
            list: Mutex::new(Vec::new()),
            running: Mutex::new(HashMap::new()),
            folders: Mutex::new((BTreeMap::new(), true)),
            dirty: AtomicBool::new(false),
            seq: AtomicU64::new(0),
            client: reqwest::Client::new(),
            persist: false,
        }));
        let path = std::env::var("CLIP").unwrap();
        let segs = vec![
            crate::clip::Seg { start: 8.0, end: 14.0, audio: Some(0), sub: None },
            crate::clip::Seg { start: 8.0, end: 14.0, audio: Some(1), sub: Some(1) },
        ];
        let dir = std::env::temp_dir().join("ofa-clip-job");
        let id = d.add_clip(&crate::clip::Spec { path, segs, height: 720, layers: vec![crate::clip::Layer { kind: "text".into(), start: 9.0, end: 13.0, x: 0.5, y: 0.15, size: 0.07, text: "VF puis VOSTFR, test".into(), color: "#ffd84d".into(), boxed: true, path: String::new() }, crate::clip::Layer { kind: "image".into(), start: 8.0, end: 14.0, x: 0.85, y: 0.8, size: 0.2, path: std::env::var("IMG").unwrap_or_default(), ..Default::default() }], music: vec![crate::clip::Music { path: std::env::var("MUSIC").unwrap_or_default(), start: 10.0, offset: 30.0, duration: 20.0, volume: 0.8 }], volume: 0.6 }, &dir.display().to_string(), "montage.mp4");
        tauri::async_runtime::block_on(d.run(&id, Arc::new(AtomicBool::new(false))));
        let f = d.get(&id).unwrap();
        println!("{:?} {} {} o err={}", f.st, f.name, f.size, f.error);
        assert_eq!(f.st, St::Done);
    }
}
