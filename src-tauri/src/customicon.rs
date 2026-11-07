//! Icône personnalisée, propre à ce PC (jamais publiée) : l'utilisateur choisit une image,
//! on enlève les bords unis, on la recadre en carré et on l'applique à la fenêtre,
//! à l'icône de la zone de notification et aux raccourcis Bureau / menu Démarrer.

use crate::config::data_dir;
use image::{imageops::FilterType, DynamicImage, GenericImageView, Rgba};
use std::path::{Path, PathBuf};
use tauri::{image::Image, AppHandle, Manager};

fn png_path() -> PathBuf {
    data_dir().join("custom-icon.png")
}
fn ico_path() -> PathBuf {
    data_dir().join("custom-icon.ico")
}

/// Enlève les bords de la couleur du coin (blanc, transparent…), puis recadre en carré centré.
fn trim_and_square(img: DynamicImage) -> DynamicImage {
    let (w, h) = img.dimensions();
    let corner = img.get_pixel(0, 0);
    let same = |p: Rgba<u8>| {
        if corner[3] < 16 {
            return p[3] < 16;
        }
        (0..3).all(|i| (p[i] as i32 - corner[i] as i32).abs() < 18) && p[3] > 200
    };
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if !same(img.get_pixel(x, y)) {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    let img = if x1 > x0 && y1 > y0 { img.crop_imm(x0, y0, x1 - x0 + 1, y1 - y0 + 1) } else { img };
    let (w, h) = img.dimensions();
    let s = w.min(h);
    img.crop_imm((w - s) / 2, (h - s) / 2, s, s)
}

/// Coins arrondis transparents (évite les coins blancs d'une image déjà arrondie).
fn round_corners(img: DynamicImage) -> DynamicImage {
    let mut rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let r = (w.min(h) as f32 * 0.16).max(1.0);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let cx = fx.clamp(r, w as f32 - r);
            let cy = fy.clamp(r, h as f32 - r);
            let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            if d > r {
                // anticrénelage sur 1 px
                let a = (1.0 - (d - r)).clamp(0.0, 1.0);
                let p = rgba.get_pixel_mut(x, y);
                p[3] = (p[3] as f32 * a) as u8;
            }
        }
    }
    DynamicImage::ImageRgba8(rgba)
}

/// Fichier .ico contenant une image PNG 256×256 (format accepté par Windows depuis Vista).
fn write_ico(png: &[u8], path: &Path) -> std::io::Result<()> {
    let mut ico = Vec::with_capacity(png.len() + 22);
    ico.extend_from_slice(&[0, 0, 1, 0, 1, 0]); // en-tête : icône, 1 image
    ico.extend_from_slice(&[0, 0, 0, 0, 1, 0, 32, 0]); // 256×256 (0 = 256), 1 plan, 32 bits
    ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes()); // les données commencent après l'en-tête
    ico.extend_from_slice(png);
    std::fs::write(path, ico)
}

/// Raccourcis de l'app (Bureau, menu Démarrer) : icône personnalisée ou celle de l'exe.
fn update_shortcuts(icon: Option<&Path>) {
    let Ok(exe) = std::env::current_exe() else { return };
    let location = match icon {
        Some(p) => format!("{},0", p.display()),
        None => format!("{},0", exe.display()),
    };
    let script = format!(
        r#"$ws=New-Object -ComObject WScript.Shell;$d=[Environment]::GetFolderPath('Desktop');$s=[Environment]::GetFolderPath('Programs');foreach($l in @("$d\OneForAll.lnk","$s\OneForAll.lnk")){{if(Test-Path $l){{$k=$ws.CreateShortcut($l);$k.IconLocation='{}';$k.Save()}}}}"#,
        location.replace('\'', "''")
    );
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let _ = cmd.output();
    #[cfg(windows)]
    unsafe {
        // Rafraîchit les icônes de l'Explorateur
        windows_sys::Win32::UI::Shell::SHChangeNotify(0x0800_0000, 0, std::ptr::null(), std::ptr::null());
    }
}

fn apply_image(app: &AppHandle, img: &DynamicImage) {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let icon = Image::new_owned(rgba.into_raw(), w, h);
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_icon(icon.clone());
    }
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_icon(Some(icon));
    }
}

/// Au démarrage : réapplique l'icône personnalisée s'il y en a une.
pub fn apply_saved(app: &AppHandle) {
    if let Ok(img) = image::open(png_path()) {
        apply_image(app, &img);
    }
}

/// Choisit une nouvelle icône (ou `None` pour revenir à celle d'origine). Renvoie l'image en PNG (base64).
pub fn set(app: &AppHandle, source: Option<String>) -> Result<Option<String>, String> {
    match source {
        Some(src) => {
            let img = image::open(&src).map_err(|e| format!("Image illisible : {e}"))?;
            let img = round_corners(trim_and_square(img).resize_exact(256, 256, FilterType::Lanczos3));
            std::fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
            let mut png = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
            std::fs::write(png_path(), &png).map_err(|e| e.to_string())?;
            write_ico(&png, &ico_path()).map_err(|e| e.to_string())?;
            apply_image(app, &img);
            let ico = ico_path();
            std::thread::spawn(move || update_shortcuts(Some(&ico)));
            use base64::Engine;
            Ok(Some(base64::engine::general_purpose::STANDARD.encode(png)))
        }
        None => {
            let _ = std::fs::remove_file(png_path());
            let _ = std::fs::remove_file(ico_path());
            if let Some(default) = app.default_window_icon().cloned() {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.set_icon(default.clone());
                }
                if let Some(tray) = app.tray_by_id("main") {
                    let _ = tray.set_icon(Some(default));
                }
            }
            std::thread::spawn(|| update_shortcuts(None));
            Ok(None)
        }
    }
}

/// Icône personnalisée actuelle (PNG en base64), s'il y en a une.
pub fn current() -> Option<String> {
    let png = std::fs::read(png_path()).ok()?;
    use base64::Engine;
    Some(base64::engine::general_purpose::STANDARD.encode(png))
}
