<script>
  // Fenêtre « Jouer » : une ROM ouverte (double-clic ou depuis un torrent) dont l'émulateur n'est pas prêt.
  import { fade, fly } from "svelte/transition";
  import Icon from "./Icon.svelte";
  import ConsoleBadge from "./ConsoleBadge.svelte";
  import { emuDetect, openUrl } from "./api.js";
  import { fmtSize } from "./model.js";
  import { arts, ui, wantArt } from "./ui.svelte.js";
  import { EMU_ICON, chooseExe, emuById, emus, installEmu, loadEmus, playRom } from "./emu.svelte.js";

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
  const file = $derived(rom?.name ?? path?.split(/[\\/]/).pop() ?? "");
  const ext = $derived(file.match(/\.([a-z0-9]{2,4})$/i)?.[1]?.toLowerCase() ?? "");
  // « Legend of Zelda, The - Ocarina of Time (Europe) (Collector's Edition) » → « The Legend of Zelda - Ocarina of Time »
  const title = $derived.by(() => {
    let n = file.replace(/\.[a-z0-9]{2,5}$/i, "");
    if ((n.match(/[._]/g) ?? []).length > (n.match(/ /g) ?? []).length) n = n.replace(/[._]/g, " ");
    n = n.replace(/\s*[([][^)\]]*[)\]]/g, "").replace(/^\d{3,5}\s*-\s*/, "").replace(/\s+/g, " ").trim();
    const [head, ...tail] = n.split(" - ");
    const m = head.match(/^(.*), (The|A|An|Les|Le|La)$/);
    return (m ? `${m[2]} ${m[1]}` : head) + (tail.length ? ` - ${tail.join(" - ")}` : "") || file;
  });

  // Jaquette du jeu (IGDB…)
  const artKey = $derived(`play:${title.toLowerCase()}`);
  $effect(() => {
    if (title) wantArt({ artKey, kind: "game", title, year: null });
  });
  const cover = $derived(arts[artKey]?.url ?? null);
  let coverOk = $state(true);
  $effect(() => {
    cover;
    coverOk = true;
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
  <!-- En-tête : jaquette sur un fond flou de la jaquette -->
  <div class="hero">
    {#if cover && coverOk}<div class="blur" style="background-image:url('{cover}')"></div>{/if}
    <div class="shade"></div>
    <button class="x" title="Fermer (Échap)" onclick={next}><Icon name="close" size={15} /></button>
    <div class="cover">
      {#if cover && coverOk}
        <img src={cover} alt="" onerror={() => (coverOk = false)} />
      {:else}
        <span class="ph"><Icon name="game" size={34} /></span>
      {/if}
      {#if ext}<span class="badge"><ConsoleBadge format={ext} platform={emu?.consoles ?? ""} small /></span>{/if}
    </div>
    <div class="ht">
      <h2 title={title}>{title}</h2>
      <small title={file}>{file}{rom?.size ? ` · ${fmtSize(rom.size)}` : ""}</small>
    </div>
  </div>

  <div class="body">
    {#if !rom || !emus.list.length}
      <p class="note"><span class="spin"></span>Analyse du jeu…</p>
    {:else if !emu || choosing}
      <p class="note">{emu ? "Avec quel émulateur ?" : "Console non reconnue : choisis l'émulateur."}</p>
      <div class="grid">
        {#each emus.list as e}
          <button class="emu" class:on={e.id === id} onclick={() => ((picked = e.id), (choosing = false))}>
            {#if EMU_ICON[e.id]}<img src={EMU_ICON[e.id]} alt="" />{:else}<span class="eph"><Icon name="game" size={16} /></span>{/if}
            <span class="et"><b>{e.name}</b><small>{e.consoles}</small></span>
            {#if e.installed}<i class="ok" title="Installé"><Icon name="check" size={12} /></i>{/if}
          </button>
        {/each}
      </div>
    {:else}
      <div class="card">
        {#if EMU_ICON[emu.id]}<img class="eico" src={EMU_ICON[emu.id]} alt="" />{:else}<span class="eico eph"><Icon name="game" size={20} /></span>{/if}
        <div class="who">
          <b>{emu.name} <small>· {emu.consoles}</small></b>
          {#if prog}
            <span class="pmsg">{prog.msg}{prog.pct ? ` · ${prog.pct} %` : ""}</span>
            <div class="bar"><span style="width:{prog.pct ?? 0}%"></span></div>
          {:else}
            <span class:ok={emu.installed}>{emu.installed ? `Installé${emu.installed.version && emu.installed.version !== "perso" ? ` · ${emu.installed.version}` : ""}` : "Pas encore installé : OneForAll l'installe pour toi"}</span>
          {/if}
        </div>
        {#if !prog}<button class="link" onclick={() => (choosing = true)}>Changer</button>{/if}
      </div>
      {#if emu.needs}<p class="needs"><Icon name="alert" size={14} />{emu.needs}</p>{/if}

      <div class="actions">
        {#if emu.installed}
          <button class="btn primary big" onclick={go}><Icon name="play" size={17} />Jouer</button>
        {:else if emu.manual}
          <button class="btn" onclick={pickExe}><Icon name="folder" size={14} />Choisir {emu.name}.exe…</button>
          <button class="btn primary big" onclick={() => openUrl(emu.page)}><Icon name="open" size={15} />Télécharger sur le site</button>
        {:else}
          <button class="btn ghost" disabled={!!prog} onclick={pickExe} title="Tu l'as déjà sur ton PC ? Indique son .exe">J'ai déjà {emu.name}</button>
          <button class="btn primary big" disabled={!!prog} onclick={installAndPlay}>
            {#if prog}<span class="spin w"></span>Installation…{:else}<Icon name="play" size={17} />Installer et jouer{/if}
          </button>
        {/if}
      </div>
    {/if}
    {#if ui.roms.length > 1}<p class="more">Encore {ui.roms.length - 1} jeu{ui.roms.length > 2 ? "x" : ""} après celui-ci</p>{/if}
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 150; background: var(--scrim); backdrop-filter: blur(3px); }
  .modal { position: fixed; z-index: 151; left: 50%; top: 50%; translate: -50% -50%; width: min(540px, 94vw); border-radius: 20px; overflow: hidden; background: var(--bg-2); border: 1px solid var(--line-strong); box-shadow: var(--shadow-lg); }

  .hero { position: relative; display: flex; align-items: flex-end; gap: 18px; padding: 26px 24px 20px; min-height: 150px; background: linear-gradient(135deg, #1e2235, #2c3150); color: white; overflow: hidden; }
  .blur { position: absolute; inset: -30px; background-size: cover; background-position: center; filter: blur(28px) saturate(1.4); opacity: 0.75; transform: scale(1.1); }
  .shade { position: absolute; inset: 0; background: linear-gradient(180deg, rgb(0 0 0 / 0.1), rgb(0 0 0 / 0.55)); }
  .x { position: absolute; z-index: 2; top: 12px; right: 12px; display: grid; place-items: center; width: 30px; height: 30px; border-radius: 9px; border: 0; background: rgb(255 255 255 / 0.16); color: white; cursor: pointer; backdrop-filter: blur(6px); }
  .x:hover { background: rgb(255 255 255 / 0.28); }
  .cover { position: relative; z-index: 1; flex: none; width: 108px; aspect-ratio: 3 / 4; border-radius: 12px; overflow: visible; }
  .cover img, .ph { width: 100%; height: 100%; border-radius: 12px; object-fit: cover; box-shadow: 0 12px 30px rgb(0 0 0 / 0.45), 0 0 0 1px rgb(255 255 255 / 0.12); }
  .ph { display: grid; place-items: center; background: rgb(255 255 255 / 0.1); color: rgb(255 255 255 / 0.7); }
  .badge { position: absolute; left: 50%; bottom: -9px; translate: -50% 0; }
  .ht { position: relative; z-index: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px; padding-bottom: 4px; padding-right: 20px; }
  h2 { margin: 0; font-size: 21px; line-height: 1.22; font-weight: 800; text-shadow: 0 2px 10px rgb(0 0 0 / 0.4); overflow: hidden; display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; }
  .ht small { font-size: 11.5px; color: rgb(255 255 255 / 0.72); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  .body { display: flex; flex-direction: column; gap: 14px; padding: 20px 22px 22px; }
  .note { margin: 0; display: flex; align-items: center; gap: 8px; font-size: 13px; color: var(--muted); }
  .card { display: flex; align-items: center; gap: 14px; padding: 12px 14px; border-radius: 14px; background: var(--surface); border: 1px solid var(--line); }
  .eico { flex: none; width: 48px; height: 48px; border-radius: 12px; object-fit: contain; }
  .eph { display: grid; place-items: center; background: var(--chip); color: var(--muted); }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .who b { font-size: 15px; }
  .who b small { font-size: 12px; font-weight: 500; color: var(--muted); }
  .who span { font-size: 12.5px; color: var(--muted); }
  .who span.ok { color: var(--ok); font-weight: 600; }
  .pmsg { font-variant-numeric: tabular-nums; }
  .bar { height: 6px; border-radius: 6px; background: var(--chip); overflow: hidden; }
  .bar span { display: block; height: 100%; background: var(--accent); border-radius: 6px; transition: width 0.2s; }
  .link { border: 0; background: none; color: var(--accent); font: inherit; font-size: 12.5px; font-weight: 650; cursor: pointer; padding: 6px 8px; border-radius: 8px; }
  .link:hover { background: var(--accent-soft); }
  .needs { margin: 0; display: flex; gap: 8px; align-items: flex-start; font-size: 12.5px; line-height: 1.45; color: var(--warn); }
  .needs :global(svg) { flex: none; margin-top: 2px; }
  .actions { display: flex; justify-content: flex-end; align-items: center; gap: 8px; flex-wrap: wrap; }
  .big { height: 44px; padding: 0 24px; font-size: 14.5px; font-weight: 700; border-radius: 12px; gap: 8px; }
  .ghost { border-color: transparent; background: none; color: var(--muted); }
  .ghost:hover:not(:disabled) { color: var(--accent); }

  .grid { display: grid; grid-template-columns: repeat(2, 1fr); gap: 8px; max-height: 320px; overflow-y: auto; }
  .emu { position: relative; display: flex; align-items: center; gap: 10px; padding: 9px 10px; border-radius: 12px; border: 1.5px solid var(--line); background: var(--surface); color: var(--text); font: inherit; text-align: left; cursor: pointer; }
  .emu:hover, .emu.on { border-color: var(--accent); }
  .emu img, .emu .eph { flex: none; width: 34px; height: 34px; border-radius: 9px; object-fit: contain; }
  .et { min-width: 0; display: flex; flex-direction: column; }
  .et b { font-size: 13px; }
  .et small { font-size: 11px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .emu .ok { position: absolute; top: 7px; right: 8px; color: var(--ok); }
  .more { margin: 0; font-size: 12px; color: var(--muted); text-align: right; }
  .spin { width: 14px; height: 14px; border-radius: 50%; border: 2px solid var(--line-strong); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
  .spin.w { border-color: rgb(255 255 255 / 0.4); border-top-color: white; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
