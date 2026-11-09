<script>
  // Fenêtre « Jouer » : une ROM ouverte (double-clic ou depuis un torrent) dont l'émulateur n'est pas prêt.
  import { fade, fly } from "svelte/transition";
  import Icon from "./Icon.svelte";
  import { emuDetect, openUrl } from "./api.js";
  import { fmtSize } from "./model.js";
  import { ui } from "./ui.svelte.js";
  import { chooseExe, emuById, emus, installEmu, loadEmus, playRom } from "./emu.svelte.js";

  const path = $derived(ui.roms[0]);
  let rom = $state(null);
  let picked = $state(null); // émulateur choisi à la main
  let choosing = $state(false);

  $effect(() => {
    const p = path;
    rom = null;
    picked = null;
    choosing = false;
    if (!emus.list.length) loadEmus();
    emuDetect(p).then((r) => p === path && (rom = r));
  });

  const id = $derived(picked ?? rom?.emu ?? null);
  const emu = $derived(id ? emuById(id) : null);
  const prog = $derived(id ? emus.progress[id] : null);
  const title = $derived.by(() => {
    const n = (rom?.name ?? path?.split(/[\\/]/).pop() ?? "").replace(/\.[a-z0-9]{2,5}$/i, "");
    return (n.match(/[._]/g) ?? []).length > (n.match(/ /g) ?? []).length ? n.replace(/[._]/g, " ") : n;
  });

  const next = () => (ui.roms = ui.roms.slice(1));

  async function go() {
    if (await playRom(path, id)) next();
  }
  async function installAndPlay() {
    const info = await installEmu(id);
    if (info?.installed) go();
  }
  async function pickExe() {
    const info = await chooseExe(id);
    if (info?.installed) go();
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && next()} />

<div class="backdrop" transition:fade={{ duration: 140 }} onclick={next} role="presentation"></div>
<div class="modal" transition:fly={{ y: 16, duration: 200 }} role="dialog" aria-label="Jouer">
  <button class="x" title="Fermer (Échap)" onclick={next}><Icon name="close" size={15} /></button>
  <div class="head">
    <span class="ic"><Icon name="game" size={26} /></span>
    <div class="ht">
      <small>{emu ? emu.consoles : "Jeu"}{rom?.size ? ` · ${fmtSize(rom.size)}` : ""}</small>
      <h2 title={rom?.name}>{title}</h2>
    </div>
  </div>

  {#if !rom || !emus.list.length}
    <p class="note">Analyse du fichier…</p>
  {:else if !emu || choosing}
    <p class="note">{emu ? "Avec quel émulateur ?" : "Console non reconnue : choisis l'émulateur."}</p>
    <div class="grid">
      {#each emus.list as e}
        <button class="emu" class:on={e.id === id} onclick={() => ((picked = e.id), (choosing = false))}>
          <b>{e.name}</b><span>{e.consoles}</span>
          {#if e.installed}<i class="ok"><Icon name="check" size={12} /></i>{/if}
        </button>
      {/each}
    </div>
  {:else}
    <div class="card">
      <div class="who">
        <b>{emu.name}</b>
        <span>{emu.installed ? `Installé${emu.installed.version && emu.installed.version !== "perso" ? ` (${emu.installed.version})` : ""}` : "Pas encore installé"}</span>
      </div>
      <button class="link" onclick={() => (choosing = true)}>Changer</button>
    </div>
    {#if emu.needs}<p class="needs"><Icon name="alert" size={14} />{emu.needs}</p>{/if}

    {#if prog}
      <div class="prog">
        <span>{prog.msg} {prog.pct ? `${prog.pct} %` : ""}</span>
        <div class="bar"><span style="width:{prog.pct}%"></span></div>
      </div>
    {/if}

    <div class="actions">
      {#if emu.installed}
        <button class="btn primary big" onclick={go}><Icon name="play" size={15} />Jouer</button>
      {:else if emu.manual}
        <button class="btn" onclick={pickExe}><Icon name="folder" size={14} />Choisir {emu.name}.exe…</button>
        <button class="btn primary" onclick={() => openUrl(emu.page)}><Icon name="open" size={14} />Télécharger sur le site</button>
      {:else}
        <button class="btn" disabled={!!prog} onclick={pickExe} title="Tu l'as déjà sur ton PC ? Indique son .exe">J'ai déjà {emu.name}</button>
        <button class="btn primary big" disabled={!!prog} onclick={installAndPlay}><Icon name="download" size={15} />Installer {emu.name} et jouer</button>
      {/if}
    </div>
  {/if}
  {#if ui.roms.length > 1}<p class="more">Encore {ui.roms.length - 1} jeu{ui.roms.length > 2 ? "x" : ""} après celui-ci</p>{/if}
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 150; background: var(--scrim); backdrop-filter: blur(3px); }
  .modal { position: fixed; z-index: 151; left: 50%; top: 50%; translate: -50% -50%; width: min(520px, 94vw); padding: 22px; border-radius: 18px; background: var(--bg-2); border: 1px solid var(--line-strong); box-shadow: var(--shadow-lg); display: flex; flex-direction: column; gap: 14px; }
  .x { position: absolute; top: 12px; right: 12px; display: grid; place-items: center; width: 28px; height: 28px; border-radius: 8px; border: 0; background: var(--chip); color: var(--muted); cursor: pointer; }
  .head { display: flex; align-items: center; gap: 14px; padding-right: 30px; }
  .ic { flex: none; display: grid; place-items: center; width: 54px; height: 54px; border-radius: 15px; background: rgb(22 163 74 / 0.12); color: #16a34a; }
  .ht { min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .ht small { font-size: 12px; font-weight: 650; color: var(--muted); }
  h2 { margin: 0; font-size: 18px; line-height: 1.25; overflow: hidden; text-overflow: ellipsis; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; }
  .note { margin: 0; font-size: 13px; color: var(--muted); }
  .card { display: flex; align-items: center; gap: 10px; padding: 10px 12px; border-radius: 11px; background: var(--chip); }
  .who { flex: 1; display: flex; flex-direction: column; gap: 2px; }
  .who b { font-size: 14px; }
  .who span { font-size: 12px; color: var(--muted); }
  .link { border: 0; background: none; color: var(--accent); font: inherit; font-size: 12.5px; font-weight: 650; cursor: pointer; }
  .needs { margin: 0; display: flex; gap: 8px; align-items: flex-start; font-size: 12.5px; line-height: 1.45; color: var(--warn); }
  .needs :global(svg) { flex: none; margin-top: 2px; }
  .prog { display: flex; flex-direction: column; gap: 6px; font-size: 12px; color: var(--muted); }
  .bar { height: 6px; border-radius: 6px; background: var(--chip); overflow: hidden; }
  .bar span { display: block; height: 100%; background: var(--accent); transition: width 0.2s; }
  .actions { display: flex; justify-content: flex-end; gap: 8px; flex-wrap: wrap; }
  .big { height: 38px; padding: 0 18px; font-weight: 650; }
  .grid { display: grid; grid-template-columns: repeat(2, 1fr); gap: 6px; }
  .emu { position: relative; display: flex; flex-direction: column; align-items: flex-start; gap: 2px; padding: 9px 11px; border-radius: 10px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: inherit; text-align: left; cursor: pointer; }
  .emu:hover, .emu.on { border-color: var(--accent); }
  .emu b { font-size: 13px; }
  .emu span { font-size: 11.5px; color: var(--muted); }
  .emu .ok { position: absolute; top: 8px; right: 8px; color: var(--ok); }
  .more { margin: 0; font-size: 12px; color: var(--muted); text-align: right; }
</style>
