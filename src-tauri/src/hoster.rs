//! Liens d'hébergeurs (1fichier, DataNodes…) : une petite fenêtre de navigateur dans l'app fait les
//! clics « téléchargement gratuit » à la place de l'utilisateur, bloque les pop-ups de pub, puis
//! récupère le fichier dès que le site le donne et l'envoie dans l'onglet DL.
//! Les vérifications (captcha, ouo.io) restent à faire par l'utilisateur : rien n'est contourné.

use tauri::webview::{DownloadEvent, NewWindowResponse};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

/// Hébergeurs reconnus (liens qui ouvrent cette fenêtre plutôt que le navigateur)
pub const HOSTS: &[&str] = &[
    "1fichier.com", "datanodes.to", "ouo.io", "ouo.press", "multiup.", "filekeeper", "gofile.io", "buzzheavier", "mediafire.com", "pixeldrain",
];

pub fn is_host(url: &str) -> bool {
    let u = url.to_lowercase();
    u.starts_with("http") && HOSTS.iter().any(|h| u.contains(h))
}

/// Clics automatiques, page par page (le script est relancé à chaque page de la fenêtre)
const AUTO: &str = r##"
(() => {
  if (window.__ofa) return; window.__ofa = 1;
  const H = location.hostname;
  const ouo = /(^|\.)ouo\.(io|press)$/.test(H);

  // ---------- bandeau OneForAll (bleu : en cours ; orange : attente imposée par le site) ----------
  let text = ouo
    ? "OneForAll : fais la vérification ouo.io, la suite se fait toute seule."
    : "OneForAll s'occupe des clics… Si une vérification apparaît, fais-la : le fichier partira tout seul dans l'app.";
  let warn = false;
  const banner = () => {
    if (!document.body) return;
    let d = document.getElementById("ofa-banner");
    if (!d) {
      d = document.createElement("div");
      d.id = "ofa-banner";
      document.body.appendChild(d);
    }
    if (d.textContent !== text) d.textContent = text;
    d.style.cssText = `position:fixed;z-index:2147483647;left:50%;top:12px;transform:translateX(-50%);background:${warn ? "#c2410c" : "#0067c0"};color:#fff;font:600 13px 'Segoe UI',sans-serif;padding:10px 18px;border-radius:12px;box-shadow:0 8px 24px rgba(0,0,0,.3);pointer-events:none;max-width:90vw;text-align:center;line-height:1.45`;
  };

  // ---------- pubs : pas de pop-up, pas d'iframe de pub, pas de faux « Download ready » ----------
  try { window.open = () => null; } catch {}
  const CAPTCHA = /recaptcha|hcaptcha|turnstile|challenges\.cloudflare|captcha/i;
  const KEEP = "#dlb, #method_free, #downloadbtn, #downloadButton, form, #ofa-banner";
  const cleanAds = () => {
    for (const f of document.querySelectorAll("iframe")) {
      if (!CAPTCHA.test(f.src || "") && !CAPTCHA.test(f.title || "")) f.remove();
    }
    // Bandeau cookies : on accepte (le site en a besoin), sinon il cache la page
    const accept = [...document.querySelectorAll("a, button")].find((e) => /fully accept|j'accepte|accept all|tout accepter|^accept(er)?$/i.test((e.textContent || "").trim()));
    if (accept && !accept.dataset.ofa) { accept.dataset.ofa = 1; accept.click(); }
    // Calques flottants par-dessus la page (pubs, faux boutons) : retirés, sauf captcha et bouton du site
    for (const el of document.querySelectorAll("body > *, body > * > *")) {
      if (el.id === "ofa-banner" || el.matches?.(KEEP) || el.querySelector?.(KEEP)) continue;
      const cs = getComputedStyle(el);
      if ((cs.position === "fixed" || cs.position === "absolute") && Number(cs.zIndex) >= 50) {
        if (el.querySelector("iframe") && CAPTCHA.test(el.innerHTML)) continue;
        el.remove();
      }
    }
    // Faux boutons « Download ready / Continuer » des régies
    for (const el of document.querySelectorAll("div, section, aside")) {
      const t = el.textContent || "";
      if (t.length < 200 && /download ready|open to continue|your download is ready/i.test(t) && !el.querySelector(KEEP)) el.remove();
    }
  };

  // ---------- clics ----------
  const done = new WeakSet();
  const visible = (el) => el && !el.disabled && el.getClientRects().length > 0;
  const click = (el) => {
    if (!visible(el) || done.has(el)) return false;
    done.add(el);
    el.click();
    return true;
  };
  const byText = (sel, re) => [...document.querySelectorAll(sel)].find((e) => re.test((e.value || e.textContent || "").trim()));
  const step = () => {
    cleanAds();
    if (!ouo) {
      if (/(^|\.)1fichier\.com$/.test(H)) {
        // Attente imposée entre deux téléchargements gratuits : on le dit, et on clique dès que c'est permis
        const w = (document.body.innerText || "").match(/(?:must wait|devez attendre)\s*(\d+)\s*min/i);
        if (w) {
          warn = true;
          text = `1fichier impose ${w[1]} min d'attente entre deux téléchargements gratuits. Laisse cette fenêtre ouverte : le fichier partira tout seul à la fin. (Ou prends un autre hébergeur, comme DataNodes.)`;
        }
        // Page 2 : le lien du fichier ; page 1 : « Accès gratuit »
        const go = [...document.querySelectorAll("a")].find((a) => /\/\/[a-z0-9-]+\.1fichier\.com\/c/i.test(a.href));
        if (go && !done.has(go)) { done.add(go); location.href = go.href; return; }
        click(document.querySelector("#dlb")) || click(byText("form input[type=submit], form button", /acc[eè]s gratuit|^free download$|t[ée]l[ée]charger/i));
      } else if (/(^|\.)datanodes\.to$/.test(H)) {
        // Le bouton se débloque à la fin du compte à rebours
        click(document.querySelector("#method_free")) ||
          click(document.querySelector("#downloadbtn")) ||
          click(byText("form button, form input[type=submit]", /download|t[ée]l[ée]charger|continue/i));
      } else if (/mediafire\.com$/.test(H)) {
        click(document.querySelector("#downloadButton, a.input.popsok"));
      } else {
        click(document.querySelector("#downloadButton, #download-button, #download_button")) ||
          click(byText("form button, form input[type=submit]", /^\s*(free\s+)?download\b|t[ée]l[ée]charger/i));
      }
    }
    banner();
  };
  setInterval(() => { try { step(); } catch {} }, 1000);
})();
"##;

/// Jeu (titre, console, jaquette) à qui appartient le fichier, pour l'onglet DL
#[derive(Clone, Default)]
pub struct Meta {
    pub title: String,
    pub platform: String,
    pub thumb: String,
}

pub fn open(app: &tauri::AppHandle, direct: crate::direct::Direct, url: &str, meta: Meta) -> Result<(), String> {
    let parsed: tauri::Url = url.parse().map_err(|_| "Lien invalide".to_string())?;
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let label = format!("host-{n}");
    let host = parsed.host_str().unwrap_or("").trim_start_matches("www.").to_string();

    let app_dl = app.clone();
    let label_dl = label.clone();
    let app_nw = app.clone();
    let label_nw = label.clone();
    WebviewWindowBuilder::new(app, &label, WebviewUrl::External(parsed))
        .title(format!("OneForAll · {host}"))
        .inner_size(1040.0, 760.0)
        .center()
        .initialization_script(AUTO)
        // Pop-ups : refusés (pubs), sauf si c'est un hébergeur, qu'on ouvre alors ici même
        .on_new_window(move |u, _| {
            if is_host(u.as_str()) {
                if let Some(w) = app_nw.get_webview_window(&label_nw) {
                    let _ = w.navigate(u);
                }
            }
            NewWindowResponse::Deny
        })
        // Le site donne le fichier : on le prend dans l'onglet DL et on ferme la fenêtre
        .on_download(move |_, ev| {
            if let DownloadEvent::Requested { url, .. } = ev {
                let u = url.to_string();
                if meta.title.is_empty() {
                    direct.add(&u);
                } else {
                    direct.add_game(&u, &meta.title, &meta.platform, &meta.thumb);
                }
                let _ = app_dl.emit("host-download", &u);
                let (a, l) = (app_dl.clone(), label_dl.clone());
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    if let Some(w) = a.get_webview_window(&l) {
                        let _ = w.close();
                    }
                });
            }
            false
        })
        .build()
        .map_err(|e| format!("Fenêtre impossible à ouvrir : {e}"))?;
    Ok(())
}
