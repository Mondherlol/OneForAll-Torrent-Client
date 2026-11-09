//! Client minimal pour l'API web de qBittorrent (v4.x et v5.x).

use crate::config::Config;
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct RawTorrent {
    pub hash: String,
    pub infohash_v1: String,
    pub infohash_v2: String,
    pub name: String,
    pub category: String,
    pub tags: String,
    pub progress: f64,
    pub state: String,
    pub size: i64,
    pub total_size: i64,
    pub completed: i64,
    pub dlspeed: i64,
    pub upspeed: i64,
    pub eta: i64,
    pub added_on: i64,
    pub completion_on: i64,
    pub save_path: String,
    pub content_path: String,
    pub ratio: f64,
    pub num_seeds: i64,
    pub num_leechs: i64,
    /// Sources complètes / incomplètes dans tout l'essaim (selon les trackers)
    pub num_complete: i64,
    pub num_incomplete: i64,
    /// Nombre de copies complètes disponibles chez les pairs connectés
    pub availability: f64,
    pub seq_dl: bool,
    pub f_l_piece_prio: bool,
    pub auto_tmm: bool,
    pub has_metadata: bool,
    pub magnet_uri: String,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
#[serde(default)]
pub struct TorrentFile {
    pub index: i64,
    pub name: String,
    pub size: i64,
    pub progress: f64,
    pub priority: i64,
    pub availability: f64,
    /// Coché mais en attente de son tour (épisodes téléchargés un par un, dans l'ordre)
    pub queued: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
#[serde(default)]
pub struct Category {
    pub name: String,
    #[serde(rename = "savePath")]
    pub save_path: String,
}

#[derive(Clone)]
struct Creds {
    url: String,
    username: String,
    password: String,
}

pub struct Qbit {
    client: Client,
    /// Modifiables à chaud (configuration automatique de qBittorrent)
    creds: std::sync::RwLock<Creds>,
}

impl Qbit {
    pub fn new(cfg: &Config) -> Self {
        let client = Client::builder()
            .cookie_store(true)
            .timeout(Duration::from_secs(8))
            .build()
            .expect("client HTTP");
        let q = Self {
            client,
            creds: std::sync::RwLock::new(Creds { url: String::new(), username: String::new(), password: String::new() }),
        };
        q.set_creds(cfg);
        q
    }

    pub fn set_creds(&self, cfg: &Config) {
        *self.creds.write().unwrap() = Creds {
            url: cfg.qbit_url.trim_end_matches('/').to_string(),
            username: cfg.username.clone(),
            password: cfg.password.clone(),
        };
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.creds.read().unwrap().url, path)
    }

    async fn login(&self) -> Result<(), String> {
        let c = self.creds.read().unwrap().clone();
        let res = self
            .client
            .post(self.url("/api/v2/auth/login"))
            .header("Referer", &c.url)
            .form(&[("username", &c.username), ("password", &c.password)])
            .send()
            .await
            .map_err(net_err)?;
        let body = res.text().await.unwrap_or_default();
        if body.trim() == "Ok." {
            Ok(())
        } else {
            Err("qBittorrent a refusé les identifiants (voir %APPDATA%\\OneForAll\\config.json)".into())
        }
    }

    /// Envoie la requête ; en cas de 403 (session expirée), se reconnecte et réessaie une fois.
    async fn send(&self, build: impl Fn(&Client) -> RequestBuilder) -> Result<Response, String> {
        let res = build(&self.client).send().await.map_err(net_err)?;
        if res.status() != StatusCode::FORBIDDEN {
            return Ok(res);
        }
        self.login().await?;
        build(&self.client).send().await.map_err(net_err)
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T, String> {
        let url = self.url(path);
        let res = self.send(|c| c.get(&url).query(query)).await?;
        if !res.status().is_success() {
            return Err(format!("qBittorrent a répondu {}", res.status()));
        }
        res.json().await.map_err(|e| format!("Réponse illisible : {e}"))
    }

    async fn get_text(&self, path: &str) -> Result<String, String> {
        let url = self.url(path);
        let res = self.send(|c| c.get(&url)).await?;
        res.text().await.map_err(|e| e.to_string())
    }

    async fn post(&self, path: &str, form: &[(&str, &str)]) -> Result<StatusCode, String> {
        let url = self.url(path);
        let res = self.send(|c| c.post(&url).form(form)).await?;
        Ok(res.status())
    }

    async fn post_ok(&self, path: &str, form: &[(&str, &str)]) -> Result<(), String> {
        check(self.post(path, form).await?)
    }

    /// Essaie le chemin qBit 5 puis l'ancien (qBit 4) s'il n'existe pas.
    async fn post_compat(&self, v5: &str, v4: &str, form: &[(&str, &str)]) -> Result<(), String> {
        match self.post(v5, form).await? {
            StatusCode::NOT_FOUND => check(self.post(v4, form).await?),
            s => check(s),
        }
    }

    pub async fn torrents(&self) -> Result<Vec<RawTorrent>, String> {
        self.get_json("/api/v2/torrents/info", &[]).await
    }

    /// `hashes` peut valoir `["all"]` pour cibler tous les torrents.
    pub async fn action(&self, hashes: &[String], action: &str) -> Result<(), String> {
        let h = hashes.join("|");
        let h = h.as_str();
        match action {
            "pause" => self.post_compat("/api/v2/torrents/stop", "/api/v2/torrents/pause", &[("hashes", h)]).await,
            "resume" => self.post_compat("/api/v2/torrents/start", "/api/v2/torrents/resume", &[("hashes", h)]).await,
            "recheck" => self.post_ok("/api/v2/torrents/recheck", &[("hashes", h)]).await,
            "delete" => self.post_ok("/api/v2/torrents/delete", &[("hashes", h), ("deleteFiles", "false")]).await,
            "delete_files" => self.post_ok("/api/v2/torrents/delete", &[("hashes", h), ("deleteFiles", "true")]).await,
            "toggle_seq" => self.post_ok("/api/v2/torrents/toggleSequentialDownload", &[("hashes", h)]).await,
            "toggle_flpp" => self.post_ok("/api/v2/torrents/toggleFirstLastPiecePrio", &[("hashes", h)]).await,
            other => Err(format!("Action inconnue : {other}")),
        }
    }

    pub async fn files(&self, hash: &str) -> Result<Vec<TorrentFile>, String> {
        self.get_json("/api/v2/torrents/files", &[("hash", hash)]).await
    }

    pub async fn set_file_priority(&self, hash: &str, ids: &[i64], priority: i64) -> Result<(), String> {
        let ids = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("|");
        let p = priority.to_string();
        self.post_ok("/api/v2/torrents/filePrio", &[("hash", hash), ("id", &ids), ("priority", &p)]).await
    }

    pub async fn categories(&self) -> Result<HashMap<String, Category>, String> {
        self.get_json("/api/v2/torrents/categories", &[]).await
    }

    /// Crée la catégorie, ou met à jour son dossier si elle existe déjà.
    pub async fn upsert_category(&self, name: &str, path: &str) -> Result<(), String> {
        let cats = self.categories().await?;
        match cats.get(name) {
            Some(c) if same_path(&c.save_path, path) => Ok(()),
            Some(_) => self.post_ok("/api/v2/torrents/editCategory", &[("category", name), ("savePath", path)]).await,
            None => self.post_ok("/api/v2/torrents/createCategory", &[("category", name), ("savePath", path)]).await,
        }
    }

    pub async fn set_category(&self, hashes: &[String], category: &str) -> Result<(), String> {
        let h = hashes.join("|");
        self.post_ok("/api/v2/torrents/setCategory", &[("hashes", &h), ("category", category)]).await
    }

    /// Ajoute un fichier .torrent local. `stopped` : ajouté sans démarrer (pour choisir les fichiers).
    pub async fn add_file(&self, name: &str, bytes: Vec<u8>, stopped: bool) -> Result<(), String> {
        let url = self.url("/api/v2/torrents/add");
        let flag = if stopped { "true" } else { "false" };
        let res = self
            .send(|c| {
                let part = reqwest::multipart::Part::bytes(bytes.clone())
                    .file_name(name.to_string())
                    .mime_str("application/x-bittorrent")
                    .expect("type MIME");
                let form = reqwest::multipart::Form::new()
                    .part("torrents", part)
                    .text("stopped", flag) // qBittorrent 5
                    .text("paused", flag); // qBittorrent 4
                c.post(&url).multipart(form)
            })
            .await?;
        check_add(res).await
    }

    /// Ajoute un lien magnet qui s'arrête tout seul dès que la liste des fichiers est connue.
    pub async fn add_magnet_staged(&self, uri: &str) -> Result<(), String> {
        let url = self.url("/api/v2/torrents/add");
        let res = self.send(|c| c.post(&url).form(&[("urls", uri), ("stopCondition", "MetadataReceived")])).await?;
        check_add(res).await
    }

    /// Déplace les fichiers (le torrent passe en gestion manuelle).
    pub async fn set_location(&self, hashes: &[String], location: &str) -> Result<(), String> {
        let h = hashes.join("|");
        self.set_auto_tmm(hashes, false).await?;
        self.post_ok("/api/v2/torrents/setLocation", &[("hashes", &h), ("location", location)]).await
    }

    pub async fn set_auto_tmm(&self, hashes: &[String], enable: bool) -> Result<(), String> {
        let h = hashes.join("|");
        let e = if enable { "true" } else { "false" };
        self.post_ok("/api/v2/torrents/setAutoManagement", &[("hashes", &h), ("enable", e)]).await
    }

    pub async fn trackers(&self, hash: &str) -> Result<serde_json::Value, String> {
        self.get_json("/api/v2/torrents/trackers", &[("hash", hash)]).await
    }

    pub async fn peers(&self, hash: &str) -> Result<serde_json::Value, String> {
        self.get_json("/api/v2/sync/torrentPeers", &[("hash", hash), ("rid", "0")]).await
    }

    /// Contenu du fichier .torrent (qBittorrent ≥ 4.5)
    pub async fn export(&self, hash: &str) -> Result<Vec<u8>, String> {
        let url = self.url("/api/v2/torrents/export");
        let res = self.send(|c| c.get(&url).query(&[("hash", hash)])).await?;
        if !res.status().is_success() {
            return Err(format!("Export impossible ({})", res.status()));
        }
        res.bytes().await.map(|b| b.to_vec()).map_err(|e| e.to_string())
    }

    /// qBittorrent répond-il ?
    pub async fn alive(&self) -> bool {
        let url = self.url("/api/v2/app/version");
        self.client.get(&url).timeout(Duration::from_secs(2)).send().await.is_ok()
    }

    /// Ferme proprement qBittorrent (il sauvegarde l'état de ses torrents).
    pub async fn shutdown(&self) -> Result<(), String> {
        self.post_ok("/api/v2/app/shutdown", &[]).await
    }

    /// Le partage (seed) est-il autorisé ? (désactivé = ratio max 0 → arrêt dès la fin du téléchargement)
    pub async fn seeding_enabled(&self) -> Result<bool, String> {
        let p: serde_json::Value = self.get_json("/api/v2/app/preferences", &[]).await?;
        let off = p["max_ratio_enabled"].as_bool() == Some(true) && p["max_ratio"].as_f64() == Some(0.0);
        Ok(!off)
    }

    pub async fn set_seeding(&self, enabled: bool) -> Result<(), String> {
        let prefs = if enabled {
            serde_json::json!({ "max_ratio_enabled": false, "max_seeding_time_enabled": false })
        } else {
            // act 0 = arrêter le torrent quand la limite est atteinte
            serde_json::json!({ "max_ratio_enabled": true, "max_ratio": 0, "max_seeding_time_enabled": true, "max_seeding_time": 0, "max_ratio_act": 0 })
        };
        let s = prefs.to_string();
        self.post_ok("/api/v2/app/setPreferences", &[("json", s.as_str())]).await
    }

    /// (téléchargement, envoi) en octets/s, 0 = illimité
    pub async fn limits(&self) -> Result<(i64, i64), String> {
        let dl = self.get_text("/api/v2/transfer/downloadLimit").await?;
        let up = self.get_text("/api/v2/transfer/uploadLimit").await?;
        Ok((dl.trim().parse().unwrap_or(0), up.trim().parse().unwrap_or(0)))
    }

    pub async fn set_limits(&self, dl: i64, up: i64) -> Result<(), String> {
        self.post_ok("/api/v2/transfer/setDownloadLimit", &[("limit", &dl.max(0).to_string())]).await?;
        self.post_ok("/api/v2/transfer/setUploadLimit", &[("limit", &up.max(0).to_string())]).await
    }
}

async fn check_add(res: Response) -> Result<(), String> {
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    if status.is_success() && !body.trim().eq_ignore_ascii_case("Fails.") {
        Ok(())
    } else {
        Err("qBittorrent a refusé ce torrent (déjà présent ou lien invalide)".into())
    }
}

fn same_path(a: &str, b: &str) -> bool {
    let n = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_lowercase();
    n(a) == n(b)
}

fn check(s: StatusCode) -> Result<(), String> {
    if s.is_success() {
        Ok(())
    } else {
        Err(format!("qBittorrent a répondu {s}"))
    }
}

fn net_err(e: reqwest::Error) -> String {
    if e.is_connect() || e.is_timeout() {
        "qBittorrent est injoignable. Vérifie qu'il est lancé et que l'interface web est activée.".into()
    } else {
        format!("Erreur réseau : {e}")
    }
}
