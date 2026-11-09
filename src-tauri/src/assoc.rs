//! Associe les fichiers .torrent et les liens magnet: à OneForAll (pour l'utilisateur courant,
//! sans droits administrateur).

#[cfg(windows)]
pub fn register() {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let Ok(exe) = std::env::current_exe() else { return };
    let exe = exe.display().to_string();
    // Seulement pour la version installée (pas pour une version de développement)
    if !exe.to_lowercase().contains(r"\oneforall\") {
        return;
    }
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let command = format!("\"{exe}\" \"%1\"");
    let icon = format!("\"{exe}\",0");
    // Icônes des fichiers (document coloré + logo + bande « TORRENT » / « JEU »).
    // Nom versionné : l'Explorateur garde en cache l'ancienne image d'un même chemin.
    let file_icon = |name: &str, bytes: &[u8]| -> String {
        let dir = crate::config::data_dir().join("icons");
        let p = dir.join(format!("{name}-v1.ico"));
        if !p.is_file() {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(&p, bytes);
        }
        if p.is_file() { format!("\"{}\"", p.display()) } else { icon.clone() }
    };
    let torrent_icon = file_icon("torrent", include_bytes!("../icons/file-torrent.ico"));
    let rom_icon = file_icon("rom", include_bytes!("../icons/file-rom.ico"));

    let set = |path: &str, name: &str, value: &str| {
        if let Ok((k, _)) = hkcu.create_subkey(path) {
            let _ = k.set_value(name, &value.to_string());
        }
    };

    // Anciennes entrées de « MediaDeck » (l'ancien nom de l'app)
    let _ = hkcu.delete_subkey_all(r"Software\Classes\MediaDeck.torrent");
    let _ = hkcu.delete_subkey_all(r"Software\MediaDeck");
    if let Ok(k) = hkcu.open_subkey_with_flags(r"Software\RegisteredApplications", winreg::enums::KEY_SET_VALUE) {
        let _ = k.delete_value("MediaDeck");
    }

    // Type de fichier OneForAll
    set(r"Software\Classes\OneForAll.torrent", "", "Fichier torrent");
    set(r"Software\Classes\OneForAll.torrent\DefaultIcon", "", &torrent_icon);
    set(r"Software\Classes\OneForAll.torrent\shell\open\command", "", &command);
    set(r"Software\Classes\.torrent", "", "OneForAll.torrent");
    set(r"Software\Classes\.torrent\OpenWithProgids", "OneForAll.torrent", "");

    // Liens magnet:
    set(r"Software\Classes\magnet", "", "URL:Lien magnet");
    set(r"Software\Classes\magnet", "URL Protocol", "");
    set(r"Software\Classes\magnet\DefaultIcon", "", &icon);
    set(r"Software\Classes\magnet\shell\open\command", "", &command);

    // ROM de jeux : jouées dans le bon émulateur
    set(r"Software\Classes\OneForAll.rom", "", "Jeu (ROM)");
    set(r"Software\Classes\OneForAll.rom\DefaultIcon", "", &rom_icon);
    set(r"Software\Classes\OneForAll.rom\shell\open", "", "Jouer avec OneForAll");
    set(r"Software\Classes\OneForAll.rom\shell\open\command", "", &command);
    for e in crate::emu::ASSOC_DEFAULT {
        set(&format!(r"Software\Classes\.{e}"), "", "OneForAll.rom");
        set(&format!(r"Software\Classes\.{e}\OpenWithProgids"), "OneForAll.rom", "");
        set(r"Software\OneForAll\Capabilities\FileAssociations", &format!(".{e}"), "OneForAll.rom");
    }
    // Trop génériques (.iso, .cue, .bin) : seulement dans « Ouvrir avec »
    for e in crate::emu::ASSOC_OPEN_WITH {
        set(&format!(r"Software\Classes\.{e}\OpenWithProgids"), "OneForAll.rom", "");
    }

    // Apparaît dans Paramètres > Applications par défaut
    set(r"Software\OneForAll\Capabilities", "ApplicationName", "OneForAll");
    set(r"Software\OneForAll\Capabilities", "ApplicationDescription", "Interface moderne pour vos torrents");
    set(r"Software\OneForAll\Capabilities\FileAssociations", ".torrent", "OneForAll.torrent");
    set(r"Software\OneForAll\Capabilities\URLAssociations", "magnet", "magnet");
    set(r"Software\RegisteredApplications", "OneForAll", r"Software\OneForAll\Capabilities");

    // Le choix « toujours ouvrir avec qBittorrent » prime sur tout : on le retire, mais
    // seulement s'il désigne qBittorrent (on ne touche pas à un autre choix de l'utilisateur).
    let fe = r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.torrent";
    let choice: String = hkcu.open_subkey(format!(r"{fe}\UserChoice")).and_then(|k| k.get_value("ProgId")).unwrap_or_default();
    if choice.to_lowercase().contains("qbittorrent") {
        let _ = hkcu.delete_subkey_all(format!(r"{fe}\UserChoiceLatest"));
        // La clé est protégée par une règle « refuser » : on la retire (la clé appartient à l'utilisateur)
        let script = format!(
            r#"$k=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('{fe}\UserChoice','ReadWriteSubTree','ChangePermissions');$a=$k.GetAccessControl();$a.Access|?{{$_.AccessControlType -eq 'Deny'}}|%{{[void]$a.RemoveAccessRule($_)}};$k.SetAccessControl($a);$k.Close();Remove-Item 'HKCU:\{fe}\UserChoice' -Force"#
        );
        let mut cmd = std::process::Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
        let _ = cmd.output();
    }
    let magnet_choice: String = hkcu
        .open_subkey(r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\magnet\UserChoice")
        .and_then(|k| k.get_value("ProgId"))
        .unwrap_or_default();
    if magnet_choice.to_lowercase().contains("qbittorrent") {
        let _ = hkcu.delete_subkey_all(r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\magnet\UserChoice");
    }

    // Prévient l'Explorateur que les associations ont changé
    unsafe {
        windows_sys::Win32::UI::Shell::SHChangeNotify(0x0800_0000, 0, std::ptr::null(), std::ptr::null());
    }
}

#[cfg(not(windows))]
pub fn register() {}

/// Liens magnet et chemins .torrent parmi les arguments de lancement.
pub fn torrent_args(args: &[String]) -> (Vec<String>, Vec<String>) {
    let mut magnets = Vec::new();
    let mut files = Vec::new();
    for a in args {
        let a = a.trim_matches('"');
        if a.to_lowercase().starts_with("magnet:") {
            magnets.push(a.to_string());
        } else if a.to_lowercase().ends_with(".torrent") && std::path::Path::new(a).is_file() {
            files.push(a.to_string());
        }
    }
    (magnets, files)
}
