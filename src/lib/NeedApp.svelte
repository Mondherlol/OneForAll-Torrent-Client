<script>
  // Il manque un lecteur (VLC pour les vidéos, SumatraPDF pour les BD / mangas / livres) :
  // OneForAll l'installe (version portable, sans droits admin) puis ouvre le fichier.
  import { fade, fly } from "svelte/transition";
  import { listen } from "@tauri-apps/api/event";
  import Icon from "./Icon.svelte";
  import { appInstall } from "./api.js";
  import { need } from "./need.svelte.js";
  import { flash } from "./ui.svelte.js";
  import icoVlc from "../assets/emus/vlc.png";
  import icoReader from "../assets/emus/reader.png";

  const APPS = {
    vlc: { name: "VLC", ico: icoVlc, what: "Il faut un lecteur vidéo pour regarder ça.", desc: "Le lecteur vidéo qui lit tout : sous-titres, pistes audio, reprise. OneForAll s'en sert pour suivre tes épisodes.", act: "Installer VLC et lire" },
    reader: { name: "SumatraPDF", ico: icoReader, what: "Il faut un lecteur pour lire ça.", desc: "Léger et rapide : BD, mangas, comics et livres (CBZ, CBR, EPUB, MOBI, PDF).", act: "Installer et lire" },
  };
  const app = $derived(APPS[need.app]);
  let prog = $state(null);

  const close = () => {
    if (prog) return;
    need.app = null;
    need.retry = null;
  };
  async function go() {
    const id = need.app;
    prog = { pct: 0, msg: "Préparation…" };
    const un = await listen("app-progress", (e) => e.payload.id === id && (prog = e.payload));
    try {
      await appInstall(id);
      const retry = need.retry;
      need.app = null;
      need.retry = null;
      retry?.();
    } catch (e) {
      flash(String(e));
    } finally {
      un();
      prog = null;
    }
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && close()} />

<div class="backdrop" transition:fade={{ duration: 140 }} onclick={close} role="presentation"></div>
<div class="modal" transition:fly={{ y: 16, duration: 200 }} role="dialog" aria-label="Installer {app.name}">
  <button class="x" title="Fermer (Échap)" onclick={close} disabled={!!prog}><Icon name="close" size={15} /></button>
  <img class="ico" src={app.ico} alt="" />
  <h2>{app.what}</h2>
  <p class="desc"><b>{app.name}</b> · {app.desc}</p>
  {#if prog}
    <div class="prog">
      <span>{prog.msg}{prog.pct ? ` · ${prog.pct} %` : ""}</span>
      <div class="bar"><span style="width:{prog.pct ?? 0}%"></span></div>
    </div>
  {/if}
  <button class="btn primary big" disabled={!!prog} onclick={go}>
    {#if prog}<span class="spin"></span>Installation…{:else}<Icon name="play" size={17} />{app.act}{/if}
  </button>
  <small class="note">Version portable, installée pour toi seul (sans droits administrateur).</small>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 150; background: var(--scrim); backdrop-filter: blur(3px); }
  .modal { position: fixed; z-index: 151; left: 50%; top: 50%; translate: -50% -50%; width: min(420px, 92vw); padding: 30px 26px 22px; border-radius: 22px; background: var(--bg-2); border: 1px solid var(--line-strong); box-shadow: var(--shadow-lg); display: flex; flex-direction: column; align-items: center; gap: 12px; text-align: center; }
  .x { position: absolute; top: 12px; right: 12px; display: grid; place-items: center; width: 30px; height: 30px; border-radius: 9px; border: 0; background: var(--chip); color: var(--muted); cursor: pointer; }
  .ico { width: 84px; height: 84px; object-fit: contain; filter: drop-shadow(0 8px 18px rgb(0 0 0 / 0.2)); }
  h2 { margin: 4px 0 0; font-size: 19px; line-height: 1.3; }
  .desc { margin: 0; font-size: 13.5px; line-height: 1.5; color: var(--muted); }
  .desc b { color: var(--text); }
  .prog { width: 100%; display: flex; flex-direction: column; gap: 6px; font-size: 12.5px; color: var(--muted); font-variant-numeric: tabular-nums; }
  .bar { height: 7px; border-radius: 7px; background: var(--chip); overflow: hidden; }
  .bar span { display: block; height: 100%; background: var(--accent); border-radius: 7px; transition: width 0.2s; }
  .big { width: 100%; height: 46px; margin-top: 6px; justify-content: center; font-size: 15px; font-weight: 700; border-radius: 13px; gap: 8px; }
  .note { font-size: 11.5px; color: var(--muted); }
  .spin { width: 15px; height: 15px; border-radius: 50%; border: 2px solid rgb(255 255 255 / 0.4); border-top-color: white; animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
