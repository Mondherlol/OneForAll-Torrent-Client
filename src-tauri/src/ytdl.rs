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
        _ => "Max",
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
pub fn args(url: &str, fmt: &str, dir: &str, tmp: &Path, ffmpeg: &Path) -> Vec<String> {
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
    a.push("-o".into());
    a.push(if plain { "%(title).150B.%(ext)s".into() } else { "%(title).120B [%(id)s].%(ext)s".into() });
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
    } else if let Some(t) = field("OFAF ") {
        Line::File(t)
    } else if let Some(e) = line.strip_prefix("ERROR:") {
        Line::Error(e.trim().to_string())
    } else {
        Line::Other
    }
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
        assert!(matches!(parse("OFA 1024 2048"), Line::Progress(1024, 2048)));
        assert!(matches!(parse("OFA 1024 NA"), Line::Progress(1024, -1)));
        assert!(matches!(parse("OFAS NA"), Line::Other));
        assert!(explain("ERROR: [youtube] x: Private video").1);
        assert!(!explain("Unable to download webpage: timed out").1);
    }
}
