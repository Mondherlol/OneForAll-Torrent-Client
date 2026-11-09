//! Émulateurs : quel émulateur pour quelle ROM, installation depuis les releases officielles
//! (dans %APPDATA%\OneForAll\emulators) et lancement d'un jeu.

use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

enum Src {
    /// Release GitHub (dépôt, motif du nom de l'archive Windows)
    Github(&'static str, &'static str),
    /// Release Forgejo (URL de l'API, motif)
    Forgejo(&'static str, &'static str),
    /// Pas de téléchargement automatique possible : page officielle + choix de l'exe
    Manual,
}

pub struct Emu {
    pub id: &'static str,
    pub name: &'static str,
    pub consoles: &'static str,
    /// Extensions propres à cet émulateur (les ambiguës — iso, cue, bin, chd, pbp — sont devinées)
    exts: &'static [&'static str],
    src: Src,
    /// Début du nom de l'exe (en minuscules)
    exe: &'static str,
    pub page: &'static str,
    /// Ce qu'il faut fournir soi-même
    pub needs: &'static str,
}

pub const EMUS: &[Emu] = &[
    Emu { id: "melonds", name: "melonDS", consoles: "Nintendo DS", exts: &["nds", "dsi", "srl"], src: Src::Github("melonDS-emu/melonDS", r"windows-x86_64\.zip$"), exe: "melonds", page: "https://melonds.kuribo64.net", needs: "" },
    Emu { id: "mgba", name: "mGBA", consoles: "Game Boy, Game Boy Color, GBA", exts: &["gba", "gb", "gbc", "sgb"], src: Src::Github("mgba-emu/mgba", r"win64\.7z$"), exe: "mgba", page: "https://mgba.io", needs: "" },
    Emu { id: "azahar", name: "Azahar", consoles: "Nintendo 3DS", exts: &["3ds", "cci", "cia", "cxi", "3dsx", "z3dsx", "zcci", "zcia"], src: Src::Github("azahar-emu/azahar", r"^azahar-windows-msvc-[\d.]+\.zip$"), exe: "azahar", page: "https://azahar-emu.org", needs: "Les jeux doivent être déchiffrés (ROM « decrypted »)." },
    Emu { id: "duckstation", name: "DuckStation", consoles: "PlayStation 1", exts: &["ecm", "mds", "psexe", "psf"], src: Src::Github("stenzek/duckstation", r"windows-x64-release\.zip$"), exe: "duckstation-qt", page: "https://www.duckstation.org", needs: "Un BIOS PS1 est nécessaire : DuckStation te le demande au premier lancement." },
    Emu { id: "pcsx2", name: "PCSX2", consoles: "PlayStation 2", exts: &[], src: Src::Github("PCSX2/pcsx2", r"windows-x64-Qt\.7z$"), exe: "pcsx2-qt", page: "https://pcsx2.net", needs: "Un BIOS PS2 est nécessaire : PCSX2 te le demande au premier lancement." },
    Emu { id: "ppsspp", name: "PPSSPP", consoles: "PSP", exts: &["cso", "prx"], src: Src::Github("hrydgard/ppsspp", r"Windows-x64\.zip$"), exe: "ppssppwindows64", page: "https://www.ppsspp.org", needs: "" },
    Emu { id: "dolphin", name: "Dolphin", consoles: "GameCube, Wii", exts: &["rvz", "gcm", "gcz", "wbfs", "wia", "ciso", "dol"], src: Src::Manual, exe: "dolphin", page: "https://dolphin-emu.org/download/", needs: "Télécharge Dolphin sur son site, décompresse-le, puis indique Dolphin.exe." },
    Emu { id: "ryujinx", name: "Ryujinx", consoles: "Nintendo Switch", exts: &["nsp", "xci", "nsz", "xcz", "nro"], src: Src::Forgejo("https://git.ryujinx.app/api/v1/repos/projects/Ryubing/releases?limit=5", r"win_x64\.zip$"), exe: "ryujinx", page: "https://ryujinx.app", needs: "Il faut tes clés (prod.keys) et le firmware de ta Switch : Ryujinx te guide au premier lancement." },
];

/// Ouvertes par OneForAll par défaut (double-clic)
pub const ASSOC_DEFAULT: &[&str] = &["nds", "dsi", "gba", "gb", "gbc", "3ds", "cci", "cia", "cxi", "3dsx", "cso", "chd", "pbp", "rvz", "gcz", "wbfs", "wia", "nsp", "xci", "nsz", "xcz"];
/// Seulement proposées dans « Ouvrir avec » (trop génériques pour les prendre par défaut)
pub const ASSOC_OPEN_WITH: &[&str] = &["iso", "cue", "bin"];
const AMBIGUOUS: &[&str] = &["iso", "cue", "bin", "chd", "pbp", "img"];

pub fn get(id: &str) -> Option<&'static Emu> {
    EMUS.iter().find(|e| e.id == id)
}

fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn is_rom(p: &Path) -> bool {
    let e = ext(p);
    AMBIGUOUS.contains(&e.as_str()) || EMUS.iter().any(|m| m.exts.contains(&e.as_str()))
}

// ---------- quel émulateur ? ----------

fn read_at(f: &mut std::fs::File, at: u64, n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    if f.seek(SeekFrom::Start(at)).is_err() {
        return Vec::new();
    }
    let got = f.read(&mut buf).unwrap_or(0);
    buf.truncate(got);
    buf
}

fn has(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Image disque (.iso, .bin) : GameCube/Wii, PSP, PS2 ou PS1 d'après son contenu.
fn sniff_disc(p: &Path) -> Option<&'static str> {
    let mut f = std::fs::File::open(p).ok()?;
    let head = read_at(&mut f, 0, 0x20);
    if head.len() >= 0x20 && (head[0x18..0x1C] == [0x5D, 0x1C, 0x9E, 0xA3] || head[0x1C..0x20] == [0xC2, 0x33, 0x9F, 0x3D]) {
        return Some("dolphin");
    }
    // Les répertoires et SYSTEM.CNF sont au début du disque
    let mut data = Vec::new();
    let _ = f.seek(SeekFrom::Start(0));
    let _ = f.take(8 << 20).read_to_end(&mut data);
    if has(&data, b"PSP_GAME") || has(&data, b"UMD_DATA") {
        return Some("ppsspp");
    }
    if has(&data, b"BOOT2") {
        return Some("pcsx2");
    }
    if has(&data, b"cdrom:") || has(&data, b"PLAYSTATION") {
        return Some("duckstation");
    }
    let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    (size > 1 << 30).then_some("pcsx2")
}

/// .cue : on regarde la piste de données qu'elle désigne (PS1 le plus souvent).
fn sniff_cue(p: &Path) -> Option<&'static str> {
    let text = std::fs::read_to_string(p).ok()?;
    let bin = text.lines().find_map(|l| {
        let l = l.trim();
        l.to_uppercase().starts_with("FILE").then(|| l.split('"').nth(1).map(str::to_string)).flatten()
    });
    bin.and_then(|b| sniff_disc(&p.with_file_name(b))).or(Some("duckstation"))
}

/// .chd : CD → PS1, DVD → PS2 (métadonnées de l'en-tête v5).
fn sniff_chd(p: &Path) -> Option<&'static str> {
    let mut f = std::fs::File::open(p).ok()?;
    let h = read_at(&mut f, 0, 64);
    if h.len() < 56 || &h[..8] != b"MComprHD" {
        return None;
    }
    let meta = u64::from_be_bytes(h[48..56].try_into().ok()?);
    let tag = read_at(&mut f, meta, 4);
    match tag.as_slice() {
        b"DVD " => Some("pcsx2"),
        b"CHT2" | b"CHTR" | b"CHCD" | b"CHGT" | b"CHGD" => Some("duckstation"),
        _ => Some("pcsx2"),
    }
}

/// .pbp : jeu PS1 converti (PSISOIMG) ou jeu PSP.
fn sniff_pbp(p: &Path) -> Option<&'static str> {
    let mut f = std::fs::File::open(p).ok()?;
    let mut data = Vec::new();
    let _ = f.by_ref().take(4 << 20).read_to_end(&mut data);
    Some(if has(&data, b"PSISOIMG") || has(&data, b"PSTITLEIMG") { "duckstation" } else { "ppsspp" })
}

// ---------- ROM dans une archive (.zip, .7z : RomsFun, WowRoms…) ----------

fn tar() -> std::process::Command {
    let exe = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join(r"System32\tar.exe");
    let mut c = std::process::Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

pub fn is_archive(p: &Path) -> bool {
    matches!(ext(p).as_str(), "zip" | "7z")
}

/// Fichiers contenus dans l'archive
fn archive_entries(p: &Path) -> Vec<String> {
    tar().arg("-tf").arg(p).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).lines().map(str::to_string).collect()).unwrap_or_default()
}

/// Émulateur d'une archive d'après les ROM qu'elle contient (sans la décompresser)
fn detect_archive(p: &Path) -> Option<&'static Emu> {
    let entries = archive_entries(p);
    let exts: Vec<String> = entries.iter().map(|e| ext(Path::new(e))).collect();
    if let Some(m) = exts.iter().find_map(|x| EMUS.iter().find(|m| m.exts.contains(&x.as_str()))) {
        return Some(m);
    }
    // Disque PS1 en .cue/.bin : le cas le plus courant
    exts.iter().any(|x| x == "cue").then(|| get("duckstation")).flatten()
}

/// Décompresse l'archive à côté d'elle (une seule fois) et renvoie la ROM à lancer.
pub fn unpack(p: &Path) -> Result<PathBuf, String> {
    let dir = p.with_extension("");
    if !dir.is_dir() {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let out = tar().arg("-xf").arg(p).arg("-C").arg(&dir).output().map_err(|e| format!("Décompression impossible : {e}"))?;
        if !out.status.success() {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(format!("Décompression impossible : {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
    }
    find_roms(&dir).into_iter().map(|r| PathBuf::from(r.path)).next().ok_or_else(|| "Aucun jeu trouvé dans l'archive".into())
}

pub fn detect(p: &Path) -> Option<&'static Emu> {
    if is_archive(p) {
        return detect_archive(p);
    }
    let e = ext(p);
    if let Some(m) = EMUS.iter().find(|m| m.exts.contains(&e.as_str())) {
        return Some(m);
    }
    let id = match e.as_str() {
        "iso" | "bin" | "img" => sniff_disc(p),
        "cue" => sniff_cue(p),
        "chd" => sniff_chd(p),
        "pbp" => sniff_pbp(p),
        _ => None,
    }?;
    get(id)
}

// ---------- émulateurs installés ----------

#[derive(Serialize, serde::Deserialize, Clone, Default)]
pub struct Installed {
    pub exe: String,
    pub version: String,
}

fn root() -> PathBuf {
    crate::config::data_dir().join("emulators")
}
fn state_path() -> PathBuf {
    root().join("emulators.json")
}
static LOCK: Mutex<()> = Mutex::new(());

pub fn installed() -> HashMap<String, Installed> {
    let map: HashMap<String, Installed> = std::fs::read(state_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    // Exe supprimé à la main → plus installé
    map.into_iter().filter(|(_, v)| Path::new(&v.exe).is_file()).collect()
}

fn save(map: &HashMap<String, Installed>) -> Result<(), String> {
    std::fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    std::fs::write(state_path(), serde_json::to_vec_pretty(map).unwrap()).map_err(|e| e.to_string())
}

pub fn set_exe(id: &str, exe: &str, version: &str) -> Result<(), String> {
    let _g = LOCK.lock().unwrap();
    let mut map = installed();
    map.insert(id.into(), Installed { exe: exe.into(), version: version.into() });
    save(&map)
}

pub fn forget(id: &str) -> Result<(), String> {
    let _g = LOCK.lock().unwrap();
    let mut map = installed();
    map.remove(id);
    save(&map)?;
    // Seul le dossier téléchargé par OneForAll est supprimé (sauvegardes et réglages restent dans les dossiers de l'émulateur)
    let dir = root().join(id);
    if dir.exists() {
        let _ = std::fs::remove_dir_all(dir);
    }
    Ok(())
}

pub fn dir_of(id: &str) -> PathBuf {
    root().join(id)
}

#[derive(Serialize)]
pub struct EmuInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub consoles: &'static str,
    pub needs: &'static str,
    pub page: &'static str,
    pub manual: bool,
    pub installed: Option<Installed>,
}

pub fn info(m: &'static Emu, map: &HashMap<String, Installed>) -> EmuInfo {
    EmuInfo { id: m.id, name: m.name, consoles: m.consoles, needs: m.needs, page: m.page, manual: matches!(m.src, Src::Manual), installed: map.get(m.id).cloned() }
}

// ---------- installation ----------

/// Adresse et version de la dernière archive Windows.
async fn latest(client: &reqwest::Client, m: &Emu) -> Result<(String, String), String> {
    let (api, pat) = match m.src {
        Src::Github(repo, pat) => (format!("https://api.github.com/repos/{repo}/releases/latest"), pat),
        Src::Forgejo(api, pat) => (api.to_string(), pat),
        Src::Manual => return Err(format!("{} ne peut pas être téléchargé automatiquement : ouvre son site.", m.name)),
    };
    let re = regex::Regex::new(pat).unwrap();
    let v: serde_json::Value = client
        .get(&api)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("{} injoignable : {e}", m.name))?
        .json()
        .await
        .map_err(|e| format!("Réponse illisible pour {} : {e}", m.name))?;
    let releases: Vec<serde_json::Value> = match v {
        serde_json::Value::Array(a) => a.into_iter().filter(|r| !r["prerelease"].as_bool().unwrap_or(false)).collect(),
        other => vec![other],
    };
    for r in releases {
        let tag = r["tag_name"].as_str().unwrap_or("").to_string();
        for a in r["assets"].as_array().into_iter().flatten() {
            let name = a["name"].as_str().unwrap_or("");
            if re.is_match(name) {
                if let Some(url) = a["browser_download_url"].as_str() {
                    return Ok((url.to_string(), tag));
                }
            }
        }
    }
    Err(format!("Pas de version Windows de {} trouvée.", m.name))
}

/// L'exe principal dans le dossier décompressé (le nom le plus court qui correspond).
fn find_exe(dir: &Path, prefix: &str) -> Option<PathBuf> {
    walkdir::WalkDir::new(dir)
        .max_depth(4)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| {
            let n = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            n.ends_with(".exe") && n.starts_with(prefix) && !n.contains("updater") && !n.contains("uninst") && !n.contains("room")
        })
        .min_by_key(|p| (p.components().count(), p.file_name().map(|n| n.len()).unwrap_or(0)))
}

pub async fn install(m: &'static Emu, note: impl Fn(u32, &str)) -> Result<Installed, String> {
    use std::io::Write;
    let client = reqwest::Client::builder().user_agent("OneForAll/0.4").connect_timeout(Duration::from_secs(15)).build().map_err(|e| e.to_string())?;
    note(0, "Recherche de la dernière version…");
    let (url, version) = latest(&client, m).await?;
    let name = url.rsplit('/').next().unwrap_or("archive.zip").to_string();
    std::fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    let archive = root().join(format!("{}.{}", m.id, if name.ends_with(".7z") { "7z" } else { "zip" }));

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
    let dir = dir_of(m.id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // tar de Windows (libarchive) : .zip et .7z. On décompresse par-dessus (garde un éventuel mode portable).
    let (archive2, dir2) = (archive.clone(), dir.clone());
    let out = tokio::task::spawn_blocking(move || tar().arg("-xf").arg(&archive2).arg("-C").arg(&dir2).output())
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("Décompression impossible : {e}"))?;
    let _ = std::fs::remove_file(&archive);
    if !out.status.success() {
        return Err(format!("Décompression impossible : {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let exe = find_exe(&dir, m.exe).ok_or_else(|| format!("{} téléchargé, mais son .exe est introuvable.", m.name))?;
    let inst = Installed { exe: exe.display().to_string(), version };
    set_exe(m.id, &inst.exe, &inst.version)?;
    Ok(inst)
}

/// Lance le jeu dans l'émulateur (une archive est d'abord décompressée à côté).
pub fn play(rom: &Path, exe: &Path) -> Result<(), String> {
    let unpacked;
    let rom = if is_archive(rom) {
        unpacked = unpack(rom)?;
        unpacked.as_path()
    } else {
        rom
    };
    let mut c = std::process::Command::new(exe);
    c.arg(rom);
    if let Some(d) = exe.parent() {
        c.current_dir(d);
    }
    c.spawn().map(|_| ()).map_err(|e| format!("Lancement impossible : {e}"))
}

// ---------- ROM dans un dossier téléchargé ----------

#[derive(Serialize)]
pub struct Rom {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub emu: Option<&'static str>,
}

pub fn rom_info(p: &Path) -> Rom {
    Rom {
        path: p.display().to_string(),
        name: p.file_name().unwrap_or_default().to_string_lossy().into(),
        size: std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
        emu: detect(p).map(|m| m.id),
    }
}

pub fn find_roms(path: &Path) -> Vec<Rom> {
    let files: Vec<PathBuf> = if path.is_file() {
        vec![path.to_path_buf()]
    } else {
        walkdir::WalkDir::new(path).max_depth(5).into_iter().flatten().filter(|e| e.file_type().is_file()).map(|e| e.into_path()).collect()
    };
    let has_cue = files.iter().any(|p| ext(p) == "cue");
    let mut roms: Vec<Rom> = files
        .iter()
        .filter(|p| (is_rom(p) && !(has_cue && ext(p) == "bin")) || (is_archive(p) && detect_archive(p).is_some()))
        // Les .bin/.img seuls sont trop génériques : gardés seulement s'ils ressemblent à un disque de jeu
        .filter(|p| !matches!(ext(p).as_str(), "bin" | "img") || detect(p).is_some())
        .map(|p| rom_info(p))
        .collect();
    roms.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    roms.truncate(60);
    roms
}

// ---------- ROM ouvertes depuis l'Explorateur ----------

static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn rom_args(args: &[String]) -> Vec<String> {
    args.iter().map(|a| a.trim_matches('"').to_string()).filter(|a| Path::new(a).is_file() && is_rom(Path::new(a))).collect()
}

/// Lance directement si l'émulateur est installé ; sinon garde la ROM pour la fenêtre « Jouer ».
/// Renvoie vrai s'il reste des ROM à montrer.
pub fn open_or_queue(roms: Vec<String>) -> bool {
    let map = installed();
    let mut queued = false;
    for r in roms {
        let p = Path::new(&r);
        let ready = detect(p).and_then(|m| map.get(m.id)).map(|i| play(p, Path::new(&i.exe)).is_ok()).unwrap_or(false);
        if !ready {
            PENDING.lock().unwrap().push(r);
            queued = true;
        }
    }
    queued
}

pub fn take_pending() -> Vec<String> {
    std::mem::take(&mut *PENDING.lock().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_extension() {
        assert_eq!(detect(Path::new("Pokemon.nds")).map(|m| m.id), Some("melonds"));
        assert_eq!(detect(Path::new("Zelda.XCI")).map(|m| m.id), Some("ryujinx"));
        assert_eq!(detect(Path::new("Metroid.rvz")).map(|m| m.id), Some("dolphin"));
        assert!(is_rom(Path::new("game.iso")));
        assert!(!is_rom(Path::new("movie.mkv")));
    }

    #[test]
    fn roms_in_archives() {
        let dir = std::env::temp_dir().join("oneforall-emu-zip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src").join("Time Hollow (Europe).nds"), b"rom").unwrap();
        let zip = dir.join("Time Hollow.zip");
        assert!(tar().arg("-a").arg("-cf").arg(&zip).arg("-C").arg(dir.join("src")).arg(".").status().unwrap().success());
        assert_eq!(detect(&zip).map(|m| m.id), Some("melonds"));
        assert_eq!(find_roms(&dir).iter().filter(|r| r.name.ends_with(".zip")).count(), 1);
        let rom = unpack(&zip).unwrap();
        assert!(rom.ends_with("Time Hollow (Europe).nds"), "{rom:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sniffs_disc_images() {
        let dir = std::env::temp_dir().join("oneforall-emu-test");
        std::fs::create_dir_all(&dir).unwrap();
        let mut gc = vec![0u8; 0x40];
        gc[0x1C..0x20].copy_from_slice(&[0xC2, 0x33, 0x9F, 0x3D]);
        std::fs::write(dir.join("gc.iso"), &gc).unwrap();
        let mut psp = vec![0u8; 4096];
        psp[2000..2008].copy_from_slice(b"PSP_GAME");
        std::fs::write(dir.join("psp.iso"), &psp).unwrap();
        let mut ps2 = vec![0u8; 4096];
        ps2[3000..3005].copy_from_slice(b"BOOT2");
        std::fs::write(dir.join("ps2.iso"), &ps2).unwrap();
        std::fs::write(dir.join("ps2.cue"), "FILE \"ps2.iso\" BINARY\n  TRACK 01 MODE2/2352\n").unwrap();
        assert_eq!(detect(&dir.join("gc.iso")).map(|m| m.id), Some("dolphin"));
        assert_eq!(detect(&dir.join("psp.iso")).map(|m| m.id), Some("ppsspp"));
        assert_eq!(detect(&dir.join("ps2.iso")).map(|m| m.id), Some("pcsx2"));
        assert_eq!(detect(&dir.join("ps2.cue")).map(|m| m.id), Some("pcsx2"));
        let _ = std::fs::remove_dir_all(dir);
    }
}

