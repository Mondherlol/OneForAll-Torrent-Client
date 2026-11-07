// État d'interface partagé : menu contextuel, confirmation, notifications, affiches.
import { forgetArtwork, getArtwork } from "./api.js";

export const ui = $state({
  menu: null, // { x, y, items }
  confirm: null, // { title, text, ok, danger, resolve }
  toast: null,
});

export function openMenu(e, items) {
  e.preventDefault();
  e.stopPropagation();
  ui.menu = { x: e.clientX, y: e.clientY, items };
}

export function confirmAsk(title, text, ok = "Confirmer", danger = false) {
  return new Promise((resolve) => (ui.confirm = { title, text, ok, danger, resolve }));
}

let toastTimer;
/** Notification en bas de l'écran, avec boutons d'action facultatifs. */
export function flash(msg, actions = []) {
  ui.toast = { msg, actions };
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (ui.toast = null), actions.length ? 9000 : 3500);
}

export async function copy(text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const t = Object.assign(document.createElement("textarea"), { value: text });
    document.body.append(t);
    t.select();
    document.execCommand("copy");
    t.remove();
  }
  flash("Copié dans le presse-papiers");
}

// ---------- affiches ----------
// arts[clé] : undefined = pas encore demandé, null = rien trouvé, { url, fallback } = trouvé

export const arts = $state({});
const queue = [];
const pending = new Set();
let running = 0;

export function wantArt(g) {
  if (g.artKey in arts || pending.has(g.artKey)) return;
  pending.add(g.artKey);
  queue.push(g);
  pump();
}

function pump() {
  while (running < 4 && queue.length) {
    const g = queue.shift();
    running++;
    getArtwork(g.artKey, g.kind, g.title, g.year)
      .then((a) => (arts[g.artKey] = a ?? null))
      .catch(() => {})
      .finally(() => {
        running--;
        pending.delete(g.artKey);
        pump();
      });
  }
}

export async function refreshArt(key) {
  await forgetArtwork(key);
  delete arts[key];
}
