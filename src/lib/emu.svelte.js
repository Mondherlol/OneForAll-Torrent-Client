// Émulateurs : liste, installation (avec avancement), lancement d'un jeu.
import { listen } from "@tauri-apps/api/event";
import { open as pickFile } from "@tauri-apps/plugin-dialog";
import { emuInstall, emuList, emuPlay, emuRemove, emuSetExe } from "./api.js";
import { flash, ui } from "./ui.svelte.js";

export const emus = $state({ list: [], progress: {} });

listen("emu-progress", (ev) => {
  if (emus.progress[ev.payload.id]) emus.progress[ev.payload.id] = ev.payload;
});

export async function loadEmus() {
  try {
    emus.list = await emuList();
  } catch (e) {
    flash(String(e));
  }
}

export const emuById = (id) => emus.list.find((e) => e.id === id);

function replace(info) {
  emus.list = emus.list.map((e) => (e.id === info.id ? info : e));
}

export async function installEmu(id) {
  emus.progress[id] = { pct: 0, msg: "Préparation…" };
  try {
    const info = await emuInstall(id);
    replace(info);
    flash(`${info.name} est installé`);
    return info;
  } catch (e) {
    flash(String(e));
    return null;
  } finally {
    delete emus.progress[id];
  }
}

export async function chooseExe(id) {
  const p = await pickFile({ filters: [{ name: "Programme", extensions: ["exe"] }] });
  if (!p) return null;
  try {
    const info = await emuSetExe(id, p);
    replace(info);
    return info;
  } catch (e) {
    flash(String(e));
    return null;
  }
}

export async function removeEmu(id) {
  try {
    await emuRemove(id);
    await loadEmus();
  } catch (e) {
    flash(String(e));
  }
}

export async function playRom(path, id) {
  try {
    await emuPlay(path, id);
    flash(`Lancement dans ${emuById(id)?.name ?? "l'émulateur"}…`);
    return true;
  } catch (e) {
    flash(String(e));
    return false;
  }
}

/** Jouer : directement si l'émulateur est prêt, sinon la fenêtre « Jouer » (installation). */
export async function play(rom) {
  if (!emus.list.length) await loadEmus();
  const e = rom.emu && emuById(rom.emu);
  if (e?.installed) return playRom(rom.path, e.id);
  ui.roms = [...ui.roms, rom.path];
}
