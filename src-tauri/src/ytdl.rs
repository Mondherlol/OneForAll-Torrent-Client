//! Vidéos et musiques des sites (YouTube, X/Twitter, Instagram, TikTok…) via yt-dlp + ffmpeg.
//!
//! L'app garde sa propre copie de yt-dlp (téléchargée au premier usage, mise à jour une fois par jour :
//! les sites changent souvent et une vieille version finit par ne plus marcher). ffmpeg sert à assembler
//! image + son en HD et à convertir en MP3 : on prend celui du système s'il existe, sinon on l'installe.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const NO_WINDOW: u32 = 0x0800_0000;

const YTDLP_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
const FFMPEG_URL: &str = "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip";

/// Une seule installation / mise à jour à la fois
static TOOL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Sites reconnus d'office : le lien part directement dans yt-dlp, sans essayer de le télécharger comme un fichier.
const HOSTS: &[&str] = &[
    "youtube.com", "youtu.be", "twitter.com", "x.com", "instagram.com", "tiktok.com", "facebook.com", "fb.watch",
    "vimeo.com", "dailymotion.com", "dai.ly", "twitch.tv", "reddit.com", "redd.it", "soundcloud.com", "bilibili.com",
    "streamable.com", "threads.net", "bsky.app", "kick.com", "pinterest.com", "pinterest.fr", "snapchat.com", "bandcamp.com",
    "open.spotify.com",
];

fn host(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let h = rest.split(['/', '?', '#']).next().unwrap_or("");
    h.rsplit('@').next().unwrap_or(h).split(':').next().unwrap_or("").to_lowercase()
}

pub fn is_media_url(url: &str) -> bool {
    let h = host(url);
    HOSTS.iter().any(|s| h == *s || h.ends_with(&format!(".{s}")))
}

/// Lien de liste (playlist YouTube, album SoundCloud…) : on la déplie en un téléchargement par élément
pub fn is_playlist_url(url: &str) -> bool {
    let u = url.to_lowercase();
    (u.contains("list=") && !u.contains("v=") && !u.contains("youtu.be/")) || u.contains("/playlist") || u.contains("/sets/") || u.contains("/album/")
}

/// Formats proposés : max, 1080, 720, mp3
pub fn label(fmt: &str) -> &'static str {
    match fmt {
        "mp3" => "MP3",
        "1080" => "1080p",
        "720" => "720p",
        _ => "Auto",
    }
}

fn tools_dir() -> PathBuf {
    crate::config::data_dir().join("tools")
}

pub fn tmp_dir(id: &str) -> PathBuf {
    crate::config::data_dir().join("tmp").join(id)
}

fn command(exe: &Path) -> Command {
    let mut c = Command::new(exe);
    c.env("PYTHONIOENCODING", "utf-8").env("PYTHONUTF8", "1");
    #[cfg(windows)]
    c.creation_flags(NO_WINDOW);
    c
}

/// Télécharge un fichier en affichant l'avancement
async fn fetch(client: &reqwest::Client, url: &str, to: &Path, what: &str, note: &(dyn Fn(String) + Sync)) -> Result<(), String> {
    use std::io::Write;
    note(format!("Installation de {what}…"));
    let mut res = client.get(url).send().await.map_err(|e| format!("Installation de {what} impossible : {e}"))?;
    if !res.status().is_success() {
        return Err(format!("Installation de {what} impossible ({})", res.status()));
    }
    let total = res.content_length().unwrap_or(0);
    let tmp = to.with_extension("download");
    let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut got = 0u64;
    let mut last = 0u64;
    loop {
        let chunk = tokio::time::timeout(Duration::from_secs(30), res.chunk())
            .await
            .map_err(|_| format!("Installation de {what} : plus de réponse"))?
            .map_err(|e| format!("Installation de {what} interrompue : {e}"))?;
        let Some(b) = chunk else { break };
        f.write_all(&b).map_err(|e| e.to_string())?;
        got += b.len() as u64;
        if total > 0 && got * 100 / total != last {
            last = got * 100 / total;
            note(format!("Installation de {what}… {last} %"));
        }
    }
    drop(f);
    std::fs::rename(&tmp, to).map_err(|e| e.to_string())
}

fn age(p: &Path) -> Duration {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).unwrap_or(Duration::MAX)
}

/// Chemin de yt-dlp (installé ou mis à jour au besoin)
pub async fn ytdlp(client: &reqwest::Client, note: &(dyn Fn(String) + Sync)) -> Result<PathBuf, String> {
    let _g = TOOL_LOCK.lock().await;
    let dir = tools_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let exe = dir.join("yt-dlp.exe");
    let stamp = dir.join("yt-dlp.checked");
    if !exe.is_file() {
        fetch(client, YTDLP_URL, &exe, "yt-dlp", note).await?;
        let _ = std::fs::write(&stamp, "");
    } else if age(&stamp) > Duration::from_secs(24 * 3600) {
        note("Mise à jour de yt-dlp…".into());
        let exe2 = exe.clone();
        // Échec de la mise à jour (hors ligne…) : on continue avec la version actuelle
        let _ = tauri::async_runtime::spawn_blocking(move || command(&exe2).arg("-U").stdout(Stdio::null()).stderr(Stdio::null()).status()).await;
        let _ = std::fs::write(&stamp, "");
    }
    Ok(exe)
}

/// Dossier contenant ffmpeg.exe et ffprobe.exe
pub async fn ffmpeg(client: &reqwest::Client, note: &(dyn Fn(String) + Sync)) -> Result<PathBuf, String> {
    let _g = TOOL_LOCK.lock().await;
    let dir = tools_dir();
    let has = |d: &Path| d.join("ffmpeg.exe").is_file() && d.join("ffprobe.exe").is_file();
    if has(&dir) {
        return Ok(dir);
    }
    // Déjà sur le PC (winget, choco…)
    let mut w = Command::new("where");
    #[cfg(windows)]
    w.creation_flags(NO_WINDOW);
    if let Ok(out) = w.arg("ffmpeg").output() {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some(d) = Path::new(line.trim()).parent().filter(|d| has(d)) {
                return Ok(d.to_path_buf());
            }
        }
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let zip = dir.join("ffmpeg.zip");
    fetch(client, FFMPEG_URL, &zip, "ffmpeg (une seule fois, ~140 Mo)", note).await?;
    note("Installation de ffmpeg… extraction".into());
    let x = dir.join("ffmpeg-x");
    let _ = std::fs::remove_dir_all(&x);
    std::fs::create_dir_all(&x).map_err(|e| e.to_string())?;
    let tar = Path::new(&std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join("System32").join("tar.exe");
    let (zip2, x2) = (zip.clone(), x.clone());
    let ok = tauri::async_runtime::spawn_blocking(move || {
        let mut c = Command::new(tar);
        #[cfg(windows)]
        c.creation_flags(NO_WINDOW);
        c.arg("-xf").arg(&zip2).arg("-C").arg(&x2).status().map(|s| s.success()).unwrap_or(false)
    })
    .await
    .unwrap_or(false);
    let _ = std::fs::remove_file(&zip);
    if !ok {
        let _ = std::fs::remove_dir_all(&x);
        return Err("Extraction de ffmpeg impossible".into());
    }
    for e in walkdir::WalkDir::new(&x).into_iter().flatten() {
        let n = e.file_name().to_string_lossy().to_lowercase();
        if n == "ffmpeg.exe" || n == "ffprobe.exe" {
            let _ = std::fs::rename(e.path(), dir.join(&n));
        }
    }
    let _ = std::fs::remove_dir_all(&x);
    if has(&dir) { Ok(dir) } else { Err("ffmpeg introuvable après installation".into()) }
}

/// Arguments yt-dlp pour un format donné. Les fichiers en cours restent dans `tmp` (reprise après pause / redémarrage)
/// et n'arrivent dans `dir` qu'une fois terminés.
pub struct Job<'a> {
    pub url: &'a str,
    pub fmt: &'a str,
    pub dir: &'a str,
    pub tmp: &'a Path,
    pub ffmpeg: &'a Path,
    /// Extrait seulement : (début, fin) en secondes
    pub section: Option<(u32, u32)>,
    /// Morceau Spotify : nom de fichier imposé et durée attendue (s)
    pub track: Option<(&'a str, u32)>,
}

/// 75 → « 1m15 », 3725 → « 1h02m05 »
pub fn hms(s: u32) -> String {
    if s >= 3600 { format!("{}h{:02}m{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}m{:02}", s / 60, s % 60) }
}

pub fn args(j: &Job) -> Vec<String> {
    let (url, fmt, dir, tmp, ffmpeg) = (j.url, j.fmt, j.dir, j.tmp, j.ffmpeg);
    let mut a: Vec<String> = [
        "--ignore-config", "--no-update", "--newline", "--no-colors", "-q", "--progress", "--no-playlist", "--no-mtime",
        "--encoding", "utf-8", "--retries", "10", "--fragment-retries", "20", "--embed-metadata",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.push("--ffmpeg-location".into());
    a.push(ffmpeg.display().to_string());
    a.push("-P".into());
    a.push(format!("home:{dir}"));
    a.push("-P".into());
    a.push(format!("temp:{}", tmp.display()));
    // Sur TikTok / Insta / X, beaucoup de titres se ressemblent (« Video by … ») : l'identifiant évite les collisions
    let h = host(url);
    let plain = ["youtube.com", "youtu.be", "soundcloud.com", "bandcamp.com"].iter().any(|s| h == *s || h.ends_with(&format!(".{s}")));
    let cut = j.section.map(|(s, e)| format!(" [{}-{}]", hms(s), hms(e))).unwrap_or_default();
    let name = match j.track {
        Some((n, _)) => format!("{}{cut}.%(ext)s", n.replace('%', "%%")),
        None if plain => format!("%(title).150B{cut}.%(ext)s"),
        None => format!("%(title).120B [%(id)s]{cut}.%(ext)s"),
    };
    a.push("-o".into());
    a.push(name);
    if let Some((s, e)) = j.section {
        // Seul l'extrait est téléchargé ; coupe précise à la seconde
        a.extend(["--download-sections".into(), format!("*{s}-{e}"), "--force-keyframes-at-cuts".into()]);
    }
    if let Some((_, secs)) = j.track {
        // Recherche YouTube : le premier résultat dont la durée colle au morceau
        a.extend(["--max-downloads".into(), "1".into()]);
        if secs > 0 {
            a.extend(["--match-filter".into(), format!("duration>{} & duration<{}", secs.saturating_sub(15), secs + 15)]);
        }
    }
    let sort = |res: &str| format!("{res}fps,vcodec:h264,acodec:aac");
    match fmt {
        "mp3" => a.extend(["-f", "ba/b", "-x", "--audio-format", "mp3", "--audio-quality", "0", "--embed-thumbnail"].map(String::from)),
        "1080" | "720" => a.extend(["-f".into(), "bv*+ba/b".into(), "-S".into(), sort(&format!("res:{fmt},")), "--merge-output-format".into(), "mp4/mkv".into()]),
        _ => a.extend(["-f".into(), "bv*+ba/b".into(), "-S".into(), sort("res,"), "--merge-output-format".into(), "mp4/mkv".into()]),
    }
    for p in [
        "before_dl:OFAT %(title)s",
        "before_dl:OFAI %(thumbnail)s",
        "before_dl:OFAS %(filesize,filesize_approx)s",
        "before_dl:OFAH %(height)s",
        "after_move:OFAF %(filepath)s",
    ] {
        a.push("-O".into());
        a.push(p.into());
    }
    a.push("--progress-template".into());
    a.push("download:OFA %(progress.downloaded_bytes)s %(progress.total_bytes,progress.total_bytes_estimate)s".into());
    a.push("--".into());
    a.push(url.into());
    a
}

/// Lance yt-dlp ; chaque ligne (stdout et stderr) arrive dans le canal.
pub fn spawn(exe: &Path, args: &[String]) -> std::io::Result<(Child, std::sync::mpsc::Receiver<String>)> {
    use std::io::BufRead;
    let mut child = command(exe).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let (tx, rx) = std::sync::mpsc::channel();
    let pipes: Vec<Box<dyn std::io::Read + Send>> = vec![Box::new(child.stdout.take().unwrap()), Box::new(child.stderr.take().unwrap())];
    for p in pipes {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut r = std::io::BufReader::new(p);
            let mut buf = Vec::new();
            while r.read_until(b'\n', &mut buf).map(|n| n > 0).unwrap_or(false) {
                let line = String::from_utf8_lossy(&buf).trim_end().to_string();
                buf.clear();
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    Ok((child, rx))
}

/// Liens de chaque élément d'une playlist
pub async fn expand(exe: &Path, url: &str) -> Vec<String> {
    let (exe, url) = (exe.to_path_buf(), url.to_string());
    tauri::async_runtime::spawn_blocking(move || {
        let out = command(&exe).args(["--ignore-config", "--no-update", "--flat-playlist", "--encoding", "utf-8", "--print", "url", "--", &url]).output();
        out.map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).filter(|l| l.starts_with("http")).collect()).unwrap_or_default()
    })
    .await
    .unwrap_or_default()
}

/// Arrête yt-dlp et ses sous-processus (ffmpeg)
pub fn kill(child: &mut Child) {
    let mut c = Command::new("taskkill");
    #[cfg(windows)]
    c.creation_flags(NO_WINDOW);
    let _ = c.args(["/PID", &child.id().to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    let _ = child.kill();
    let _ = child.wait();
}

pub enum Line {
    Title(String),
    Thumb(String),
    Size(i64),
    /// Hauteur de la vidéo (1080…)
    Height(i64),
    File(String),
    /// téléchargé, total (-1 = inconnu)
    Progress(i64, i64),
    Error(String),
    Other,
}

pub fn parse(line: &str) -> Line {
    let num = |s: &str| s.parse::<f64>().map(|f| f as i64).unwrap_or(-1);
    if let Some(r) = line.strip_prefix("OFA ") {
        let mut it = r.split_whitespace();
        let done = it.next().map(num).unwrap_or(-1);
        let total = it.next().map(num).unwrap_or(-1);
        return if done >= 0 { Line::Progress(done, total) } else { Line::Other };
    }
    let field = |p: &str| line.strip_prefix(p).map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && s != "NA");
    if let Some(t) = field("OFAT ") {
        Line::Title(t)
    } else if let Some(t) = field("OFAI ") {
        Line::Thumb(t)
    } else if let Some(t) = field("OFAS ") {
        Line::Size(num(&t))
    } else if let Some(t) = field("OFAH ") {
        Line::Height(num(&t))
    } else if let Some(t) = field("OFAF ") {
        Line::File(t)
    } else if let Some(e) = line.strip_prefix("ERROR:") {
        Line::Error(e.trim().to_string())
    } else {
        Line::Other
    }
}

/// « 1080p », « 4K »…
pub fn res_label(h: i64) -> String {
    match h {
        h if h >= 2000 => "4K".into(),
        h if h >= 1400 => "1440p".into(),
        h if h > 0 => format!("{h}p"),
        _ => String::new(),
    }
}

/// Ce qu'on sait d'une vidéo avant de la télécharger (pour choisir un extrait ou un format)
#[derive(serde::Serialize, Default)]
pub struct Info {
    pub title: String,
    pub duration: f64,
    pub thumb: String,
    /// Flux lisible directement dans l'app (mp4 image + son), pour l'aperçu de l'extrait
    pub preview: String,
    /// Hauteurs disponibles, de la plus grande à la plus petite
    pub heights: Vec<i64>,
    /// En-têtes exigés par le serveur pour lire `preview` (YouTube refuse un autre User-Agent)
    #[serde(skip)]
    pub headers: Vec<(String, String)>,
}

pub async fn info(exe: &Path, url: &str) -> Result<Info, String> {
    let (exe, url) = (exe.to_path_buf(), url.to_string());
    let out = tauri::async_runtime::spawn_blocking(move || {
        command(&exe)
            .args(["--ignore-config", "--no-update", "--no-playlist", "--encoding", "utf-8", "-j", "-f"])
            .arg("b[height<=720][ext=mp4][vcodec!=none][acodec!=none]/b[ext=mp4]/b")
            .arg("--")
            .arg(&url)
            .output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("Lancement de yt-dlp impossible : {e}"))?;
    let v: serde_json::Value = match serde_json::from_slice(&out.stdout) {
        Ok(v) => v,
        Err(_) => {
            let err = String::from_utf8_lossy(&out.stderr);
            let last = err.lines().rev().find(|l| l.starts_with("ERROR:")).unwrap_or("Vidéo illisible");
            return Err(explain(last.trim_start_matches("ERROR:").trim()).0);
        }
    };
    let mut heights: Vec<i64> = v["formats"]
        .as_array()
        .map(|f| f.iter().filter(|f| f["vcodec"].as_str() != Some("none")).filter_map(|f| f["height"].as_i64()).collect())
        .unwrap_or_default();
    heights.sort_unstable_by(|a, b| b.cmp(a));
    heights.dedup();
    let preview = v["url"].as_str().filter(|_| v["vcodec"].as_str() != Some("none")).unwrap_or("").to_string();
    Ok(Info {
        title: v["title"].as_str().unwrap_or("").into(),
        duration: v["duration"].as_f64().unwrap_or(0.0),
        thumb: v["thumbnail"].as_str().unwrap_or("").into(),
        preview,
        heights,
        headers: v["http_headers"]
            .as_object()
            .map(|h| h.iter().filter(|(k, _)| k.as_str() != "Sec-Fetch-Mode").filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
            .unwrap_or_default(),
    })
}

/// Aperçus en cours : id → (flux, en-têtes)
static PREVIEWS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, (String, Vec<(String, String)>)>>> =
    std::sync::LazyLock::new(Default::default);

/// Rend l'aperçu lisible par la fenêtre de l'app : elle le lira via le protocole « ofapv », relayé ici avec les bons en-têtes
pub fn expose_preview(info: &mut Info) {
    if info.preview.is_empty() {
        return;
    }
    let id = format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    let mut map = PREVIEWS.lock().unwrap();
    if map.len() > 20 {
        map.clear();
    }
    map.insert(id.clone(), (std::mem::take(&mut info.preview), info.headers.clone()));
    info.preview = if cfg!(windows) { format!("http://ofapv.localhost/{id}") } else { format!("ofapv://localhost/{id}") };
}

/// Relais d'un aperçu : morceaux de 2 Mo au plus (la vidéo redemande la suite toute seule)
pub async fn proxy(req: tauri::http::Request<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
    use tauri::http::{Response, StatusCode};
    let fail = |s: StatusCode| Response::builder().status(s).body(Vec::new()).unwrap();
    let id = req.uri().path().trim_start_matches('/').to_string();
    let Some((url, headers)) = PREVIEWS.lock().unwrap().get(&id).cloned() else { return fail(StatusCode::NOT_FOUND) };
    const CHUNK: u64 = 2 * 1024 * 1024;
    let range = req.headers().get("range").and_then(|v| v.to_str().ok()).unwrap_or("bytes=0-").to_string();
    let (a, b) = range.trim_start_matches("bytes=").split_once('-').unwrap_or(("0", ""));
    let start: u64 = a.parse().unwrap_or(0);
    let end = b.parse::<u64>().ok().map(|e| e.min(start + CHUNK - 1)).unwrap_or(start + CHUNK - 1);
    let ua = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("user-agent")).map(|(_, v)| v.clone()).unwrap_or_default();
    let client = reqwest::Client::new();
    // Un flux tout juste créé refuse parfois la première requête (403) : quelques essais
    let mut res = None;
    for attempt in 0..5 {
        let r = client.get(&url).header("Range", format!("bytes={start}-{end}")).header("User-Agent", &ua).send().await;
        match r {
            Ok(r) if r.status().is_success() => {
                res = Some(r);
                break;
            }
            Ok(r) if r.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE => return fail(StatusCode::RANGE_NOT_SATISFIABLE),
            Ok(r) => crate::player::log(&format!("aperçu : {} (essai {})", r.status(), attempt + 1)),
            Err(e) => crate::player::log(&format!("aperçu : {e}")),
        }
        tokio::time::sleep(Duration::from_millis(600)).await;
    }
    let Some(res) = res else { return fail(StatusCode::BAD_GATEWAY) };
    let get = |k: &str| res.headers().get(k).and_then(|v| v.to_str().ok()).map(String::from);
    let ctype = get("content-type").unwrap_or_else(|| "video/mp4".into());
    let crange = get("content-range");
    let Ok(body) = res.bytes().await else { return fail(StatusCode::BAD_GATEWAY) };
    let total = crange.as_deref().and_then(|c| c.rsplit('/').next()).and_then(|t| t.parse::<u64>().ok());
    let last = start + body.len() as u64 - 1;
    let mut out = Response::builder()
        .status(StatusCode::PARTIAL_CONTENT)
        .header("Content-Type", ctype)
        .header("Accept-Ranges", "bytes")
        .header("Content-Length", body.len().to_string())
        .header("Access-Control-Allow-Origin", "*");
    out = out.header("Content-Range", format!("bytes {start}-{last}/{}", total.map(|t| t.to_string()).unwrap_or("*".into())));
    out.body(body.to_vec()).unwrap_or_else(|_| fail(StatusCode::INTERNAL_SERVER_ERROR))
}

// ---------- compression (ffmpeg) ----------

/// Durée (s), largeur, hauteur, débit audio (bit/s) d'une vidéo
#[derive(serde::Serialize, Default, Clone)]
pub struct Probe {
    pub duration: f64,
    pub width: i64,
    pub height: i64,
    pub audio: i64,
    pub size: i64,
}

pub fn probe(ffdir: &Path, path: &Path) -> Result<Probe, String> {
    let out = command(&ffdir.join("ffprobe.exe"))
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .output()
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|_| "Vidéo illisible".to_string())?;
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let video = streams.iter().find(|s| s["codec_type"] == "video").ok_or("Pas d'image dans ce fichier")?;
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");
    let n = |x: &serde_json::Value| x.as_str().and_then(|s| s.parse::<f64>().ok()).or(x.as_f64()).unwrap_or(0.0);
    Ok(Probe {
        duration: n(&v["format"]["duration"]),
        width: video["width"].as_i64().unwrap_or(0),
        height: video["height"].as_i64().unwrap_or(0),
        audio: audio.map(|a| n(&a["bit_rate"]) as i64).filter(|b| *b > 0).map(|b| b.min(128_000)).unwrap_or(if audio.is_some() { 128_000 } else { 0 }),
        size: std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0),
    })
}

/// Réglages pour viser `target` octets : (débit vidéo bit/s, hauteur de sortie, débit audio bit/s)
pub fn plan(p: &Probe, target: i64) -> (i64, i64, i64) {
    let audio = if p.audio == 0 { 0 } else if target < 12_000_000 { 64_000 } else { 96_000.min(p.audio.max(64_000)) };
    let total = (target as f64 * 8.0 * 0.95 / p.duration.max(1.0)) as i64;
    let video = (total - audio).max(80_000);
    // Moins de débit → image plus petite, sinon c'est de la bouillie
    let max_h = match video {
        v if v < 400_000 => 360,
        v if v < 900_000 => 480,
        v if v < 2_000_000 => 720,
        v if v < 4_500_000 => 1080,
        _ => 4320,
    };
    (video, p.height.min(max_h), audio)
}

pub fn compress_args(src: &Path, dst: &Path, p: &Probe, target: i64) -> Vec<String> {
    let (vb, h, ab) = plan(p, target);
    let mut a: Vec<String> = vec!["-y".into(), "-hide_banner".into(), "-nostats".into(), "-progress".into(), "pipe:1".into(), "-i".into(), src.display().to_string()];
    a.extend(["-c:v", "libx264", "-preset", "medium", "-pix_fmt", "yuv420p"].map(String::from));
    a.extend(["-b:v".into(), vb.to_string(), "-maxrate".into(), (vb * 3 / 2).to_string(), "-bufsize".into(), (vb * 2).to_string()]);
    if h < p.height {
        a.extend(["-vf".into(), format!("scale=-2:{h}")]);
    }
    if ab > 0 {
        a.extend(["-c:a".into(), "aac".into(), "-b:a".into(), ab.to_string()]);
    } else {
        a.push("-an".into());
    }
    a.extend(["-movflags".into(), "+faststart".into(), dst.display().to_string()]);
    a
}

/// Lance un exécutable du dossier ffmpeg (sortie ligne par ligne dans le canal)
pub fn spawn_ff(ffdir: &Path, args: &[String]) -> std::io::Result<(Child, std::sync::mpsc::Receiver<String>)> {
    spawn(&ffdir.join("ffmpeg.exe"), args)
}

/// Nom du site, affiché tant que le titre n'est pas connu
pub fn site(url: &str) -> String {
    let h = host(url);
    let names = [
        ("youtu", "YouTube"), ("spotify", "Spotify"), ("twitter", "X"), ("x.com", "X"), ("instagram", "Instagram"), ("tiktok", "TikTok"),
        ("facebook", "Facebook"), ("fb.watch", "Facebook"), ("soundcloud", "SoundCloud"), ("twitch", "Twitch"), ("reddit", "Reddit"),
        ("redd.it", "Reddit"), ("vimeo", "Vimeo"), ("dailymotion", "Dailymotion"),
    ];
    names.iter().find(|(k, _)| h.contains(k)).map(|(_, n)| n.to_string()).unwrap_or_else(|| h.trim_start_matches("www.").to_string())
}

// ---------- Spotify (DRM : on retrouve chaque morceau sur YouTube, en MP3) ----------

pub fn is_spotify(url: &str) -> bool {
    host(url) == "open.spotify.com"
}

/// (type, id) d'un lien open.spotify.com/(intl-fr/)track|album|playlist/ID
fn spotify_ref(url: &str) -> Option<(String, String)> {
    let path = url.split("open.spotify.com/").nth(1)?.split(['?', '#']).next()?;
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty() && !p.starts_with("intl-")).collect();
    match parts.as_slice() {
        [t, id, ..] if ["track", "album", "playlist"].contains(t) => Some((t.to_string(), id.to_string())),
        _ => None,
    }
}

pub enum Spotify {
    /// « Artiste - Titre », durée en secondes
    Track(String, u32),
    /// Liens des morceaux
    List(Vec<String>),
}

/// Lit la page « embed » publique de Spotify (pas besoin de compte)
pub async fn spotify(client: &reqwest::Client, url: &str) -> Result<Spotify, String> {
    let (kind, id) = spotify_ref(url).ok_or("Lien Spotify non pris en charge (morceau, album ou playlist uniquement).")?;
    let page = client
        .get(format!("https://open.spotify.com/embed/{kind}/{id}"))
        .send()
        .await
        .map_err(|e| format!("Spotify injoignable : {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let json = page
        .split(r#"<script id="__NEXT_DATA__" type="application/json">"#)
        .nth(1)
        .and_then(|s| s.split("</script>").next())
        .ok_or("Page Spotify illisible")?;
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let e = &v["props"]["pageProps"]["state"]["data"]["entity"];
    let secs = |d: &serde_json::Value| (d.as_u64().unwrap_or(0) / 1000) as u32;
    if kind == "track" {
        let title = e["name"].as_str().or(e["title"].as_str()).ok_or("Morceau Spotify introuvable")?;
        let artists: Vec<&str> = e["artists"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect()).unwrap_or_default();
        let name = if artists.is_empty() { title.to_string() } else { format!("{} - {title}", artists.join(", ")) };
        return Ok(Spotify::Track(name, secs(&e["duration"])));
    }
    let urls: Vec<String> = e["trackList"]
        .as_array()
        .map(|l| l.iter().filter_map(|t| t["uri"].as_str()?.strip_prefix("spotify:track:").map(|id| format!("https://open.spotify.com/track/{id}"))).collect())
        .unwrap_or_default();
    if urls.is_empty() { Err("Playlist Spotify vide ou privée.".into()) } else { Ok(Spotify::List(urls)) }
}

/// Erreur yt-dlp → (message clair, inutile de réessayer ?)
pub fn explain(err: &str) -> (String, bool) {
    let e = err.to_lowercase();
    let has = |k: &[&str]| k.iter().any(|w| e.contains(w));
    if has(&["unsupported url", "is not a valid url"]) {
        ("Lien non reconnu : ce n'est ni un fichier, ni une vidéo d'un site pris en charge (les hébergeurs type 1fichier ne le sont pas).".into(), true)
    } else if has(&["private video", "video unavailable", "is unavailable", "has been removed", "does not exist", "not available in your country", "geo restrict"]) {
        ("Vidéo indisponible (privée, supprimée ou bloquée dans ton pays).".into(), true)
    } else if has(&["sign in", "login", "log in", "cookies", "age-restricted", "confirm your age"]) {
        ("Ce contenu demande d'être connecté au site (compte privé, limite d'âge…).".into(), true)
    } else if has(&["requested format is not available", "no video formats"]) {
        ("Aucun format téléchargeable pour ce lien.".into(), true)
    } else if has(&["timed out", "getaddrinfo", "connection", "network", "http error 5", "unable to download", "read timed"]) {
        ("Connexion perdue — nouvel essai automatique".into(), false)
    } else {
        let short: String = err.chars().take(220).collect();
        (short, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_links() {
        assert!(is_media_url("https://www.youtube.com/watch?v=abc"));
        assert!(is_media_url("https://youtu.be/abc"));
        assert!(is_media_url("https://x.com/u/status/1"));
        assert!(is_media_url("https://vm.tiktok.com/xyz/"));
        assert!(!is_media_url("https://fox.com/a.mp4"));
        assert!(!is_media_url("https://example.org/x.zip"));
        assert!(is_playlist_url("https://www.youtube.com/playlist?list=PL1"));
        assert!(!is_playlist_url("https://www.youtube.com/watch?v=a&list=PL1"));
        assert!(is_spotify("https://open.spotify.com/intl-fr/track/abc?si=1"));
        assert_eq!(spotify_ref("https://open.spotify.com/intl-fr/track/abc?si=1"), Some(("track".into(), "abc".into())));
        assert_eq!(spotify_ref("https://open.spotify.com/playlist/xyz"), Some(("playlist".into(), "xyz".into())));
        assert_eq!(hms(75), "1m15");
        assert_eq!(hms(3725), "1h02m05");
        assert!(matches!(parse("OFA 1024 2048"), Line::Progress(1024, 2048)));
        assert!(matches!(parse("OFA 1024 NA"), Line::Progress(1024, -1)));
        assert!(matches!(parse("OFAS NA"), Line::Other));
        assert!(explain("ERROR: [youtube] x: Private video").1);
        assert!(!explain("Unable to download webpage: timed out").1);
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// cargo test ytdl_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn ytdl_live() {
        tauri::async_runtime::block_on(async {
            let c = reqwest::Client::new();
            let exe = ytdlp(&c, &|_| {}).await.unwrap();
            let t = std::time::Instant::now();
            let i = info(&exe, "https://www.youtube.com/watch?v=b32i4Pxkl28").await.unwrap();
            println!("info en {:?} : « {} » {} s, hauteurs {:?}, aperçu {}", t.elapsed(), i.title, i.duration, i.heights, &i.preview[..i.preview.len().min(60)]);
            let ff = ffmpeg(&c, &|_| {}).await.unwrap();
            let src = std::env::var("SRC").unwrap();
            let p = probe(&ff, Path::new(&src)).unwrap();
            let target = 10 * 1_048_576;
            println!("source {} o, {} s, {}x{} ; plan {:?}", p.size, p.duration, p.width, p.height, plan(&p, target));
            let dst = std::env::temp_dir().join("ofa-compress.mp4");
            let t = std::time::Instant::now();
            let ok = command(&ff.join("ffmpeg.exe")).args(compress_args(Path::new(&src), &dst, &p, target)).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap();
            println!("compression {:?} en {:?} → {} o", ok, t.elapsed(), std::fs::metadata(&dst).unwrap().len());
        });
    }
}

#[cfg(test)]
mod proxy_live {
    use super::*;

    /// cargo test proxy_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn proxy_live() {
        tauri::async_runtime::block_on(async {
            let exe = ytdlp(&reqwest::Client::new(), &|_| {}).await.unwrap();
            let mut i = info(&exe, "https://www.youtube.com/watch?v=b32i4Pxkl28").await.unwrap();
            expose_preview(&mut i);
            for range in ["bytes=0-", "bytes=3000000-"] {
                let req = tauri::http::Request::builder().uri(&i.preview).header("range", range).body(Vec::new()).unwrap();
                let res = proxy(req).await;
                println!("{range} → {} {:?} {} octets", res.status(), res.headers().get("content-range"), res.body().len());
                assert_eq!(res.status(), 206);
            }
        });
    }
}
