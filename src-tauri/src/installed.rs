//! Programmes installés (clés « Uninstall » du registre Windows), pour savoir si un jeu
//! ou un logiciel téléchargé est déjà installé, où, et comment le désinstaller.

use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Serialize, Clone, Debug)]
pub struct Program {
    pub name: String,
    /// Dossier d'installation (déduit de l'icône ou du désinstalleur si absent)
    pub location: String,
    #[serde(skip)]
    pub uninstall: String,
}

static CACHE: Mutex<Option<(Instant, Vec<Program>)>> = Mutex::new(None);

/// Liste des programmes installés (mise en cache 60 s).
pub fn programs() -> Vec<Program> {
    let mut c = CACHE.lock().unwrap();
    if let Some((t, list)) = c.as_ref() {
        if t.elapsed() < Duration::from_secs(60) {
            return list.clone();
        }
    }
    let list = read_registry();
    *c = Some((Instant::now(), list.clone()));
    list
}

pub fn invalidate() {
    *CACHE.lock().unwrap() = None;
}

#[cfg(windows)]
fn read_registry() -> Vec<Program> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    const PATHS: [(bool, &str); 3] = [
        (true, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        (true, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
        (false, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
    ];
    let mut out = Vec::new();
    for (machine, path) in PATHS {
        let root = RegKey::predef(if machine { HKEY_LOCAL_MACHINE } else { HKEY_CURRENT_USER });
        let Ok(key) = root.open_subkey(path) else { continue };
        for sub in key.enum_keys().flatten() {
            let Ok(k) = key.open_subkey(&sub) else { continue };
            let get = |n: &str| k.get_value::<String, _>(n).unwrap_or_default();
            let name = get("DisplayName");
            let uninstall = get("UninstallString");
            if name.is_empty() || uninstall.is_empty() || k.get_value::<u32, _>("SystemComponent").unwrap_or(0) == 1 {
                continue;
            }
            let mut location = get("InstallLocation");
            if location.is_empty() {
                location = parent_of_command(&get("DisplayIcon")).or_else(|| parent_of_command(&uninstall)).unwrap_or_default();
            }
            out.push(Program { name, location: location.trim_end_matches('\\').to_string(), uninstall });
        }
    }
    out
}

#[cfg(not(windows))]
fn read_registry() -> Vec<Program> {
    Vec::new()
}

/// « "C:\Jeu\unins000.exe" /x » → « C:\Jeu »
fn parent_of_command(cmd: &str) -> Option<String> {
    let cmd = cmd.trim();
    let exe = if let Some(rest) = cmd.strip_prefix('"') {
        rest.split('"').next()?
    } else {
        cmd.split(',').next()?.split(" /").next()?
    };
    let p = Path::new(exe.trim());
    if !p.is_absolute() {
        return None;
    }
    let mut dir = p.parent()?;
    // Désinstalleurs rangés dans un sous-dossier « Uninstall » : on remonte d'un cran
    if dir.file_name().is_some_and(|n| n.eq_ignore_ascii_case("uninstall")) {
        dir = dir.parent()?;
    }
    Some(dir.to_string_lossy().to_string())
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Programme installé correspondant au titre d'un torrent.
pub fn find<'a>(title: &str, list: &'a [Program]) -> Option<&'a Program> {
    let t = norm(title);
    if t.len() < 3 {
        return None;
    }
    list.iter().find(|p| {
        let n = norm(&p.name);
        crate::artwork::similar(title, &p.name) || (t.len() >= 6 && n.contains(&t))
    })
}

/// Lance le désinstalleur d'un programme (identifié par son nom exact dans le registre).
pub fn uninstall(name: &str) -> Result<(), String> {
    let list = programs();
    let p = list.iter().find(|p| p.name == name).ok_or("Programme introuvable dans Windows")?;
    let mut cmd = std::process::Command::new("cmd");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // « start "" … » : le désinstalleur s'ouvre normalement, sans fenêtre de console
        cmd.raw_arg(format!("/c start \"\" {}", p.uninstall));
        cmd.creation_flags(0x0800_0000);
    }
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())?;
    invalidate();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching() {
        let list = vec![
            Program { name: "Blue Prince - ElAmigos version 1.06.1".into(), location: String::new(), uninstall: String::new() },
            Program { name: "SAP PowerDesigner 16.7 SP05 (x64)".into(), location: String::new(), uninstall: String::new() },
            Program { name: "Adobe Illustrator 2026".into(), location: String::new(), uninstall: String::new() },
        ];
        assert!(find("Blue Prince", &list).is_some());
        assert!(find("PowerDesigner", &list).is_some());
        assert!(find("Adobe Illustrator", &list).is_some());
        assert!(find("GOD EATER 3", &list).is_none());
        assert_eq!(parent_of_command(r#""D:\Games\Pragmata\Uninstall\unins000.exe""#).as_deref(), Some(r"D:\Games\Pragmata"));
    }
}
