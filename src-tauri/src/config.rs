use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Lue et écrite dans %APPDATA%\OneForAll\config.json
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub qbit_url: String,
    pub username: String,
    pub password: String,
    /// Type de média (film, series, game…) → dossier par défaut
    pub folders: BTreeMap<String, String>,
    /// Range automatiquement les nouveaux torrents sans catégorie
    pub auto_sort: bool,
    /// Téléchargement séquentiel (épisode 1, puis 2…) pour les séries
    pub seq_episodes: bool,
    /// Clé API TMDB facultative (meilleures affiches de films/séries)
    pub tmdb_key: String,
    /// Lance qBittorrent à l'ouverture et le ferme à la fermeture
    pub manage_qbit: bool,
    /// Chemin de qbittorrent.exe (vide = emplacement standard)
    pub qbit_exe: String,
    /// Animés et séries dans deux catégories distinctes (sinon : tout en « séries »)
    pub split_anime: bool,
    /// Clé API Torznab de C411 (recherche), gardée uniquement sur ce PC
    pub c411_key: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            qbit_url: "http://127.0.0.1:8080".into(),
            username: "admin".into(),
            password: String::new(),
            folders: BTreeMap::new(),
            auto_sort: true,
            seq_episodes: true,
            tmdb_key: String::new(),
            manage_qbit: true,
            qbit_exe: String::new(),
            split_anime: true,
            c411_key: String::new(),
        }
    }
}

/// Réglages exposés à l'interface (sans les identifiants).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    pub folders: BTreeMap<String, String>,
    pub auto_sort: bool,
    pub seq_episodes: bool,
    pub tmdb_key: String,
    pub manage_qbit: bool,
    #[serde(default = "yes")]
    pub split_anime: bool,
    #[serde(default)]
    pub c411_key: String,
}

fn yes() -> bool {
    true
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("OneForAll")
}

/// L'app s'appelait « MediaDeck » : on reprend ses réglages et son cache au premier lancement.
pub fn migrate_old_data() {
    let new = data_dir();
    let old = new.with_file_name("MediaDeck");
    if !new.exists() && old.is_dir() {
        if std::fs::rename(&old, &new).is_err() {
            let _ = std::fs::create_dir_all(&new);
            if let Ok(entries) = std::fs::read_dir(&old) {
                for e in entries.flatten() {
                    if e.path().is_file() {
                        let _ = std::fs::copy(e.path(), new.join(e.file_name()));
                    }
                }
            }
        }
    }
}

fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

pub fn load() -> Config {
    let mut cfg: Config = std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default();
    // Toujours actifs (ce ne sont plus des options)
    cfg.auto_sort = true;
    cfg.manage_qbit = true;
    cfg
}

pub fn save(cfg: &Config) -> Result<(), String> {
    std::fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(config_path(), json).map_err(|e| e.to_string())
}

/// Nom de la catégorie qBittorrent associée à chaque type de média.
pub fn category_for(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "film" => "Films",
        "series" => "Series",
        "anime" => "Animes",
        "game" => "Jeux",
        "software" => "Logiciels",
        "book" => "Livres",
        "audio" => "Audio",
        _ => return None,
    })
}
