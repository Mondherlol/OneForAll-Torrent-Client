// Mises à jour automatiques : OneForAll vérifie les releases GitHub (signées) au démarrage.
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";

export const upd = $state({
  current: "",
  available: null, // { version, notes, date }
  checking: false,
  progress: null, // 0..1 pendant le téléchargement
  error: null,
  checkedOnce: false,
  lastCheck: 0,
});

let update = null;

getVersion().then((v) => (upd.current = v)).catch(() => {});

export async function checkUpdates() {
  upd.checking = true;
  upd.error = null;
  try {
    update = await check();
    upd.available = update ? { version: update.version, notes: update.body ?? "", date: update.date ?? "" } : null;
  } catch (e) {
    upd.error = String(e);
  } finally {
    upd.checking = false;
    upd.checkedOnce = true;
    upd.lastCheck = Date.now();
  }
  return upd.available;
}

/** Télécharge, installe (installateur en mode silencieux) puis relance l'app. */
export async function installUpdate() {
  if (!update) return;
  let total = 0;
  let got = 0;
  upd.progress = 0;
  try {
    await update.downloadAndInstall((ev) => {
      if (ev.event === "Started") total = ev.data.contentLength ?? 0;
      else if (ev.event === "Progress") {
        got += ev.data.chunkLength;
        upd.progress = total ? got / total : null;
      } else if (ev.event === "Finished") upd.progress = 1;
    });
    await relaunch();
  } catch (e) {
    upd.error = String(e);
    upd.progress = null;
  }
}
