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
}

impl Dl {
    pub fn done(&self) -> i64 {
        self.segs.iter().map(|s| s.done).sum()
    }
    fn display_name(&self) -> String {
        if self.name.is_empty() {
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

    pub fn list(&self) -> Vec<Dl> {
        self.0.list.lock().unwrap().clone()
    }

    pub fn get(&self, id: &str) -> Option<Dl> {
        self.0.list.lock().unwrap().iter().find(|d| d.id == id).cloned()
    }

    pub fn add(&self, url: &str) -> String {
        let n = self.0.seq.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let id = format!("dl-{:x}{n:x}", nanos);
        let dl = Dl { id: id.clone(), url: url.trim().to_string(), size: -1, st: St::Queued, added_on: now(), ..Default::default() };
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
                    if let Some(d) = removed.filter(|d| !d.name.is_empty()) {
                        // Un fichier inachevé ne sert à rien : il part toujours. Le fichier terminé, seulement si demandé.
                        let _ = std::fs::remove_file(d.part_path());
                        if action == "delete_files" && d.final_path().is_file() {
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
            return Err(Fail::Fatal("Ce lien ouvre une page web, pas un fichier. Les hébergeurs (1fichier, Uptobox…) ne sont pas encore pris en charge.".into()));
        }
        let (size, resumable) = if status == reqwest::StatusCode::PARTIAL_CONTENT {
            let total = RE_RANGE_TOTAL.captures(&text(reqwest::header::CONTENT_RANGE)).and_then(|c| c[1].parse().ok()).unwrap_or(-1);
            (total, total > 0)
        } else {
            let len = res.content_length().map(|l| l as i64).unwrap_or(-1);
            (len, len > 0 && text(reqwest::header::ACCEPT_RANGES).contains("bytes"))
        };
        drop(res);

        // Dossier selon le type de fichier (réglages « Dossiers »), sinon Téléchargements
        let dir = {
            let (folders, split_anime) = self.0.folders.lock().unwrap().clone();
            let mut kind = crate::media::detect_kind(&name, "").as_str();
            if kind == "anime" && (!split_anime || folders.get("anime").is_none_or(|f| f.is_empty())) {
                kind = "series";
            }
            let existing = self.get(id).map(|d| d.dir).unwrap_or_default();
            if !existing.is_empty() {
                existing
            } else {
                folders.get(kind).filter(|f| !f.is_empty()).cloned().unwrap_or_else(downloads_dir)
            }
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
        if d.segs.is_empty() || d.name.is_empty() {
            self.probe(id, &d.url).await?;
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
