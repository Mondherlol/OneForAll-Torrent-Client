//! Configuration automatique de qBittorrent au premier lancement : OneForAll a besoin de son
//! interface web (Web UI) pour lui parler. On l'active pour ce PC uniquement, avec un identifiant
//! et un mot de passe générés, enregistrés dans la config de OneForAll.

use crate::config::Config;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::path::PathBuf;
use std::time::Duration;

pub fn ini_path() -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    Some(PathBuf::from(appdata).join("qBittorrent").join("qBittorrent.ini"))
}

fn read_ini(section: &str, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(ini_path()?).ok()?;
    let mut in_section = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_section = l == format!("[{section}]");
        } else if in_section {
            if let Some(v) = l.strip_prefix(&format!("{key}=")) {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Mot de passe au format attendu par qBittorrent : PBKDF2-HMAC-SHA512, 100 000 tours, sel de 16 octets.
fn pbkdf2_hash(password: &str) -> Result<String, String> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|e| e.to_string())?;
    let mut out = [0u8; 64];
    pbkdf2::pbkdf2_hmac::<sha2::Sha512>(password.as_bytes(), &salt, 100_000, &mut out);
    Ok(format!("@ByteArray({}:{})", STANDARD.encode(salt), STANDARD.encode(out)))
}

fn random_password() -> Result<String, String> {
    const ALPHA: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut buf = [0u8; 28];
    getrandom::fill(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf.iter().map(|b| ALPHA[*b as usize % ALPHA.len()] as char).collect())
}

fn hidden(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

pub fn is_running() -> bool {
    hidden(std::process::Command::new("tasklist").args(["/FI", "IMAGENAME eq qbittorrent.exe", "/NH"]))
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains("qbittorrent.exe"))
        .unwrap_or(false)
}

/// Ferme qBittorrent : d'abord poliment, puis de force s'il ne répond pas.
pub fn close() {
    let _ = hidden(std::process::Command::new("taskkill").args(["/IM", "qbittorrent.exe"])).output();
    for _ in 0..12 {
        if !is_running() {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let _ = hidden(std::process::Command::new("taskkill").args(["/F", "/IM", "qbittorrent.exe"])).output();
    for _ in 0..10 {
        if !is_running() {
            return;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// Écrit la configuration Web UI dans qBittorrent.ini (qBittorrent doit être fermé)
/// et renvoie la config OneForAll mise à jour.
pub fn configure(mut cfg: Config) -> Result<Config, String> {
    let ini = ini_path().ok_or("Dossier APPDATA introuvable")?;
    if !ini.is_file() {
        std::fs::create_dir_all(ini.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&ini, "[Preferences]\n").map_err(|e| e.to_string())?;
    }
    let port: u16 = read_ini("Preferences", r"WebUI\Port").and_then(|p| p.trim().parse().ok()).unwrap_or(8080);
    let already_enabled = read_ini("Preferences", r"WebUI\Enabled").is_some_and(|v| v.trim() == "true");
    let username = "oneforall".to_string();
    let password = random_password()?;
    let hash = pbkdf2_hash(&password)?;

    let set = |k: &str, v: &str| crate::set_ini(&ini, "Preferences", k, v).map_err(|e| e.to_string());
    set(r"WebUI\Enabled", "true")?;
    if !already_enabled {
        // Pas d'accès depuis le réseau : uniquement depuis ce PC
        set(r"WebUI\Address", "127.0.0.1")?;
    }
    set(r"WebUI\Port", &port.to_string())?;
    set(r"WebUI\LocalHostAuth", "false")?;
    set(r"WebUI\Username", &username)?;
    set(r"WebUI\Password_PBKDF2", &format!("\"{hash}\""))?;

    cfg.qbit_url = format!("http://127.0.0.1:{port}");
    cfg.username = username;
    cfg.password = password;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    /// Même résultat que la référence .NET (Rfc2898DeriveBytes, SHA512, 100 000 tours)
    #[test]
    #[ignore]
    fn print_known_hash() {
        let salt: Vec<u8> = (1..=16).collect();
        let mut out = [0u8; 64];
        pbkdf2::pbkdf2_hmac::<sha2::Sha512>(b"abc", &salt, 100_000, &mut out);
        use base64::Engine;
        println!("HASH={}", base64::engine::general_purpose::STANDARD.encode(out));
    }

    #[test]
    fn hash_format() {
        let h = super::pbkdf2_hash("test").unwrap();
        assert!(h.starts_with("@ByteArray(") && h.ends_with(')') && h.contains(':'));
        assert_eq!(super::random_password().unwrap().len(), 28);
    }
}
