//! Lecteurs installés par OneForAll quand il en manque un : VLC (vidéos, musique) et SumatraPDF
//! (BD, mangas, livres : CBZ, CBR, CB7, EPUB, MOBI, PDF). Versions portables, dans
//! %APPDATA%\OneForAll\apps, sans droits administrateur.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Serialize, Clone)]
pub struct AppInfo {
    pub id: &'static str,
    pub name: &'static str,
    /// Ce qu'il ouvre
    pub what: &'static str,
    /// Chemin de l'exe s'il est là (installé par OneForAll ou déjà sur le PC)
    pub exe: Option<String>,
}

pub fn root() -> PathBuf {
    crate::config::data_dir().join("apps")
}

/// VLC du PC (installé normalement), sinon celui installé par OneForAll
pub fn vlc() -> Option<PathBuf> {
    let mut c = vec![];
    for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Ok(base) = std::env::var(var) {
            c.push(Path::new(&base).join("VideoLAN").join("VLC").join("vlc.exe"));
            c.push(Path::new(&base).join("Programs").join("VideoLAN").join("VLC").join("vlc.exe"));
        }
    }
    c.into_iter().find(|p| p.is_file()).or_else(|| crate::emu::find_exe(&root().join("vlc"), "vlc"))
}

/// SumatraPDF du PC, sinon celui installé par OneForAll
pub fn reader() -> Option<PathBuf> {
    let mut c = vec![];
    for var in ["ProgramFiles", "LOCALAPPDATA"] {
        if let Ok(base) = std::env::var(var) {
            c.push(Path::new(&base).join("SumatraPDF").join("SumatraPDF.exe"));
        }
    }
    c.into_iter().find(|p| p.is_file()).or_else(|| crate::emu::find_exe(&root().join("reader"), "sumatrapdf"))
}

/// YACReader (BD, mangas, comics) du PC, sinon celui installé par OneForAll
pub fn comics() -> Option<PathBuf> {
    let mut c = vec![root().join("yacreader").join("YACReader.exe")];
    for var in ["ProgramFiles", "LOCALAPPDATA"] {
        if let Ok(base) = std::env::var(var) {
            c.push(Path::new(&base).join("YACReader").join("YACReader.exe"));
            c.push(Path::new(&base).join("Programs").join("YACReader").join("YACReader.exe"));
        }
    }
    c.into_iter().find(|p| p.is_file())
}

pub fn list() -> Vec<AppInfo> {
    vec![
        AppInfo { id: "vlc", name: "VLC", what: "Vidéos et musique", exe: vlc().map(|p| p.display().to_string()) },
        AppInfo { id: "comics", name: "YACReader", what: "BD, mangas et comics (CBZ, CBR, CB7, PDF)", exe: comics().map(|p| p.display().to_string()) },
        AppInfo { id: "reader", name: "SumatraPDF", what: "Livres (EPUB, MOBI)", exe: reader().map(|p| p.display().to_string()) },
    ]
}

/// YACReader n'existe pas en portable pour Windows : son installateur (Inno Setup) en silencieux,
/// pour l'utilisateur seul, dans le dossier des apps de OneForAll
async fn install_comics(client: &reqwest::Client, note: &impl Fn(u32, &str)) -> Result<PathBuf, String> {
    use std::io::Write;
    note(0, "Recherche de la dernière version…");
    let v: serde_json::Value = client
        .get("https://api.github.com/repos/YACReader/yacreader/releases/latest")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let url = v["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a["browser_download_url"].as_str())
        .find(|u| u.contains("-winx64") && u.ends_with(".exe"))
        .ok_or("Version Windows de YACReader introuvable")?
        .to_string();
    std::fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    let setup = root().join("yacreader-setup.exe");
    let mut res = client.get(&url).send().await.map_err(|e| format!("Téléchargement impossible : {e}"))?;
    let total = res.content_length().unwrap_or(0);
    let mut f = std::fs::File::create(&setup).map_err(|e| e.to_string())?;
    let (mut got, mut last) = (0u64, u32::MAX);
    while let Some(b) = tokio::time::timeout(Duration::from_secs(30), res.chunk())
        .await
        .map_err(|_| "Téléchargement bloqué (plus de réponse)".to_string())?
        .map_err(|e| format!("Téléchargement interrompu : {e}"))?
    {
        f.write_all(&b).map_err(|e| e.to_string())?;
        got += b.len() as u64;
        let pct = if total > 0 { (got * 100 / total) as u32 } else { 0 };
        if pct != last {
            last = pct;
            note(pct, "Téléchargement…");
        }
    }
    drop(f);
    note(100, "Installation…");
    let dir = root().join("yacreader");
    let (s, d) = (setup.clone(), dir.clone());
    let status = tokio::task::spawn_blocking(move || {
        std::process::Command::new(&s)
            .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-", "/CURRENTUSER", "/NOICONS"])
            .arg(format!("/DIR={}", d.display()))
            .status()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("Installation impossible : {e}"))?;
    let _ = std::fs::remove_file(&setup);
    if !status.success() {
        return Err(format!("L'installation de YACReader a échoué (code {:?})", status.code()));
    }
    comics().ok_or_else(|| "YACReader installé, mais introuvable.".into())
}

/// Lien de la dernière version portable
async fn latest(client: &reqwest::Client, id: &str) -> Result<String, String> {
    match id {
        "vlc" => {
            let base = "https://download.videolan.org/pub/videolan/vlc/last/win64/";
            let html = client.get(base).send().await.map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
            let name = regex::Regex::new(r"vlc-[\d.]+-win64\.zip").unwrap().find(&html).map(|m| m.as_str().to_string()).ok_or("Version de VLC introuvable")?;
            Ok(format!("{base}{name}"))
        }
        "reader" => {
            let v: serde_json::Value = client
                .get("https://api.github.com/repos/sumatrapdfreader/sumatrapdf/releases/latest")
                .send()
                .await
                .map_err(|e| e.to_string())?
                .json()
                .await
                .map_err(|e| e.to_string())?;
            let ver = v["tag_name"].as_str().unwrap_or("3.6.1rel").trim_end_matches("rel").to_string();
            Ok(format!("https://files.sumatrapdfreader.org/software/sumatrapdf/rel/{ver}/SumatraPDF-{ver}-64.zip"))
        }
        _ => Err("Application inconnue".into()),
    }
}

/// Télécharge et décompresse ; `note(pourcentage, message)`
pub async fn install(id: &str, note: impl Fn(u32, &str)) -> Result<PathBuf, String> {
    use std::io::Write;
    let client = reqwest::Client::builder().user_agent("OneForAll").connect_timeout(Duration::from_secs(15)).build().map_err(|e| e.to_string())?;
    if id == "comics" {
        return install_comics(&client, &note).await;
    }
    note(0, "Recherche de la dernière version…");
    let url = latest(&client, id).await?;
    std::fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    let archive = root().join(format!("{id}.zip"));
    let mut res = client.get(&url).send().await.map_err(|e| format!("Téléchargement impossible : {e}"))?;
    if !res.status().is_success() {
        return Err(format!("Téléchargement impossible ({})", res.status()));
    }
    let total = res.content_length().unwrap_or(0);
    let mut f = std::fs::File::create(&archive).map_err(|e| e.to_string())?;
    let (mut got, mut last) = (0u64, u32::MAX);
    loop {
        let chunk = tokio::time::timeout(Duration::from_secs(30), res.chunk())
            .await
            .map_err(|_| "Téléchargement bloqué (plus de réponse)".to_string())?
            .map_err(|e| format!("Téléchargement interrompu : {e}"))?;
        let Some(b) = chunk else { break };
        f.write_all(&b).map_err(|e| e.to_string())?;
        got += b.len() as u64;
        let pct = if total > 0 { (got * 100 / total) as u32 } else { 0 };
        if pct != last {
            last = pct;
            note(pct, "Téléchargement…");
        }
    }
    drop(f);
    note(100, "Décompression…");
    let dir = root().join(id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let (a, d) = (archive.clone(), dir.clone());
    let out = tokio::task::spawn_blocking(move || crate::emu::tar().arg("-xf").arg(&a).arg("-C").arg(&d).output())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Décompression impossible : {e}"))?;
    let _ = std::fs::remove_file(&archive);
    if !out.status.success() {
        return Err(format!("Décompression impossible : {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    match id {
        "vlc" => vlc(),
        _ => reader(),
    }
    .ok_or_else(|| "Installé, mais l'exe est introuvable.".into())
}

/// Ouvre un livre (ou un dossier de livres : le premier) dans SumatraPDF
/// BD / mangas / comics dans YACReader (« NO_COMICS » s’il manque), livres EPUB / MOBI dans SumatraPDF (« NO_READER »)
pub fn read(path: &Path) -> Result<(), String> {
    let target = if path.is_dir() {
        walkdir::WalkDir::new(path)
            .max_depth(4)
            .into_iter()
            .flatten()
            .map(|e| e.into_path())
            .filter(|p| p.is_file() && matches!(p.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref(), Some("cbz" | "cbr" | "cb7" | "cbt" | "epub" | "mobi" | "pdf" | "azw3")))
            .min()
            .ok_or("Aucun livre (CBZ, CBR, EPUB, PDF…) dans ce dossier")?
    } else {
        path.to_path_buf()
    };
    let ext = target.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let exe = if matches!(ext.as_str(), "epub" | "mobi" | "azw3") { reader().ok_or("NO_READER")? } else { comics().ok_or("NO_COMICS")? };
    std::process::Command::new(exe).arg(&target).spawn().map(|_| ()).map_err(|e| format!("Lancement impossible : {e}"))
}

#[cfg(test)]
mod tests {
    /// `cargo test apps_latest -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn apps_latest() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let c = reqwest::Client::builder().user_agent("OneForAll").build().unwrap();
        for id in ["vlc", "reader"] {
            println!("{id} → {:?}", rt.block_on(super::latest(&c, id)));
        }
    }
}

#[cfg(test)]
mod install_tests {
    /// `cargo test apps_install_reader -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn apps_install_reader() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let r = rt.block_on(super::install("reader", |_, _| {}));
        println!("reader → {r:?} ; vlc → {:?}", super::vlc());
    }
}
