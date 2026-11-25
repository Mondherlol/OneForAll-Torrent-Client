//! AllForOne (optimisation PC et connexion pour le jeu) : installé et ouvert depuis OneForAll.
//! Téléchargé depuis sa dernière release GitHub, signature vérifiée avec la même clé que les
//! mises à jour de OneForAll, installé pour l'utilisateur seul (sans droits admin) dans
//! %LOCALAPPDATA%\Programs\AllForOne. Ensuite AllForOne se met à jour tout seul.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

const LATEST: &str = "https://github.com/Mondherlol/AllForOne/releases/latest/download/latest.json";
/// Même clé que tauri.conf.json > plugins.updater.pubkey
const PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDcxRjgxNzlERjVDMjhCNjUKUldSbGk4TDFuUmY0Y2YyMjdIbHU2alFMQTZlak1xUExHdlBZczJubWlpYy9HM1VPUFBBaEtDcVIK";
const EXE: &str = "AllForOne.exe";
const REG: &str = r"Software\AllForOne";

#[derive(Serialize)]
pub struct Status {
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    /// installé par OneForAll (donc désinstallable d'ici)
    pub managed: bool,
}

fn install_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("Programs").join("AllForOne")
}

/// AllForOne note son emplacement dans le registre à chaque lancement : on le retrouve où qu'il soit.
fn registered() -> (Option<PathBuf>, Option<String>) {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let Ok(k) = hkcu.open_subkey(REG) else { return (None, None) };
    let path = k.get_value::<String, _>("Path").ok().map(PathBuf::from).filter(|p| p.exists());
    let ver = path.as_ref().and_then(|_| k.get_value::<String, _>("Version").ok());
    (path, ver)
}

pub fn find() -> Option<PathBuf> {
    registered().0.or_else(|| Some(install_dir().join(EXE)).filter(|p| p.exists()))
}

pub fn status() -> Status {
    let (reg, version) = registered();
    let path = reg.or_else(|| Some(install_dir().join(EXE)).filter(|p| p.exists()));
    let managed = path.as_ref().is_some_and(|p| p.starts_with(install_dir()));
    Status { installed: path.is_some(), path: path.map(|p| p.display().to_string()), version, managed }
}

fn verify(data: &[u8], sig_b64: &str) -> Result<(), String> {
    use base64::Engine;
    let b64 = |s: &str| -> Result<String, String> {
        let raw = base64::engine::general_purpose::STANDARD.decode(s.trim()).map_err(|e| e.to_string())?;
        String::from_utf8(raw).map_err(|e| e.to_string())
    };
    let pk = minisign_verify::PublicKey::decode(&b64(PUBKEY)?).map_err(|e| e.to_string())?;
    let sig = minisign_verify::Signature::decode(&b64(sig_b64)?).map_err(|e| e.to_string())?;
    pk.verify(data, &sig, true).map_err(|_| "Signature invalide : installation annulée.".to_string())
}

/// Dossiers « Bureau » et « Menu Démarrer > Programmes » (le Bureau peut être dans OneDrive)
fn shortcut_dirs() -> Vec<PathBuf> {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let folders = hkcu.open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Shell Folders").ok();
    let get = |name: &str| folders.as_ref().and_then(|k| k.get_value::<String, _>(name).ok()).map(PathBuf::from);
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default();
    vec![
        get("Desktop").unwrap_or_else(|| home.join("Desktop")),
        get("Programs").unwrap_or_else(|| appdata.join(r"Microsoft\Windows\Start Menu\Programs")),
    ]
}

fn create_shortcuts(exe: &Path) {
    for dir in shortcut_dirs() {
        if let Ok(mut l) = mslnk::ShellLink::new(exe) {
            l.set_working_dir(exe.parent().map(|p| p.display().to_string()));
            l.set_icon_location(Some(exe.display().to_string()));
            l.set_name(Some("AllForOne — optimisation PC et connexion pour le jeu".into()));
            let _ = l.create_lnk(dir.join("AllForOne.lnk"));
        }
    }
}

/// Télécharge, vérifie et installe la dernière version ; `note(pourcentage, message)`
pub async fn install(note: impl Fn(u32, &str)) -> Result<PathBuf, String> {
    let client = reqwest::Client::builder().user_agent("OneForAll").connect_timeout(Duration::from_secs(15)).build().map_err(|e| e.to_string())?;
    note(0, "Recherche de la dernière version…");
    let res = client.get(LATEST).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("Aucune version d'AllForOne publiée ({})", res.status()));
    }
    let v: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    let p = &v["platforms"]["windows-x86_64"];
    let (url, sig) = (p["url"].as_str().unwrap_or_default(), p["signature"].as_str().unwrap_or_default());
    if url.is_empty() || sig.is_empty() {
        return Err("Release AllForOne incomplète".into());
    }

    let mut res = client.get(url).send().await.map_err(|e| format!("Téléchargement impossible : {e}"))?;
    if !res.status().is_success() {
        return Err(format!("Téléchargement impossible ({})", res.status()));
    }
    let total = res.content_length().unwrap_or(0);
    let mut data = Vec::with_capacity(total as usize);
    let mut last = u32::MAX;
    while let Some(b) = tokio::time::timeout(Duration::from_secs(30), res.chunk())
        .await
        .map_err(|_| "Téléchargement bloqué (plus de réponse)".to_string())?
        .map_err(|e| format!("Téléchargement interrompu : {e}"))?
    {
        data.extend_from_slice(&b);
        let pct = if total > 0 { (data.len() as u64 * 100 / total) as u32 } else { 0 };
        if pct != last {
            last = pct;
            note(pct, "Téléchargement…");
        }
    }
    note(100, "Vérification…");
    verify(&data, sig)?;

    note(100, "Installation…");
    let dir = install_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let exe = dir.join(EXE);
    std::fs::write(&exe, &data).map_err(|e| {
        if e.raw_os_error() == Some(32) {
            "AllForOne est ouvert : ferme-le puis réessaie.".to_string()
        } else {
            format!("Installation impossible : {e}")
        }
    })?;
    create_shortcuts(&exe);
    Ok(exe)
}

/// Lance AllForOne (il demande lui-même les droits admin à Windows)
pub fn launch() -> Result<(), String> {
    let exe = find().ok_or("AllForOne n'est pas installé.")?;
    std::process::Command::new(&exe)
        .current_dir(exe.parent().unwrap_or(Path::new(".")))
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Impossible de lancer AllForOne : {e}"))
}

/// Désinstalle la copie installée par OneForAll (exe, raccourcis, entrée du registre)
pub fn uninstall() -> Result<(), String> {
    let dir = install_dir();
    let exe = dir.join(EXE);
    if exe.exists() {
        std::fs::remove_file(&exe).map_err(|e| {
            if e.raw_os_error() == Some(32) || e.raw_os_error() == Some(5) {
                "AllForOne est ouvert : ferme-le puis réessaie.".to_string()
            } else {
                e.to_string()
            }
        })?;
    }
    let _ = std::fs::remove_dir_all(&dir);
    for d in shortcut_dirs() {
        let _ = std::fs::remove_file(d.join("AllForOne.lnk"));
    }
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let _ = hkcu.delete_subkey_all(REG);
    Ok(())
}
