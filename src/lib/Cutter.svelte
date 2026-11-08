<script>
  // Choix d'un extrait sur une timeline, avec la vidéo qui joue (aperçu en flux direct, rien n'est téléchargé)
  import { fade, scale } from "svelte/transition";
  import Icon from "./Icon.svelte";

  /** info : media_info ; start/end en secondes ; ondone({ start, end }) ; onclose() */
  let { info, start: s0 = 0, end: e0 = null, ondone, onclose } = $props();

  const D = Math.max(1, info.duration || 1);
  let start = $state(Math.min(s0, D - 1));
  let end = $state(Math.min(e0 ?? Math.min(D, s0 + 60), D));
  let cur = $state(start);
  let paused = $state(true);
  let video = $state();
  let broken = $state(!info.preview);
  let track;
  let drag = null;
  // Lecture de l'extrait seul : on s'arrête à la fin
  let inLoop = false;

  const pct = (t) => `${(t / D) * 100}%`;
  const two = (n) => String(n).padStart(2, "0");
  function fmt(t, tenth = false) {
    t = Math.max(0, t);
    const h = Math.floor(t / 3600), m = Math.floor((t % 3600) / 60), s = Math.floor(t % 60);
    const base = h ? `${h}:${two(m)}:${two(s)}` : `${m}:${two(s)}`;
    return tenth ? `${base}.${Math.floor((t % 1) * 10)}` : base;
  }
  function parse(txt) {
    const p = txt.trim().split(/[:h.,m]/).filter((x) => x !== "");
    if (!p.length || p.some((x) => !/^\d+$/.test(x))) return NaN;
    return p.reduce((s, x) => s * 60 + +x, 0);
  }

  function seek(t) {
    cur = Math.min(Math.max(0, t), D);
    if (video) video.currentTime = cur;
  }
  function at(e) {
    const r = track.getBoundingClientRect();
    return Math.min(Math.max(0, (e.clientX - r.left) / r.width), 1) * D;
  }
  function down(e, what) {
    e.preventDefault();
    e.stopPropagation();
    drag = what;
    track.setPointerCapture(e.pointerId);
    if (what === "seek") seek(at(e));
  }
  function move(e) {
    if (!drag) return;
    const t = Math.round(at(e) * 10) / 10;
    if (drag === "start") {
      start = Math.min(t, end - 1);
      seek(start);
    } else if (drag === "end") {
      end = Math.max(t, start + 1);
      seek(end);
    } else seek(t);
  }
  const up = () => (drag = null);

  function toggle() {
    if (!video || broken) return;
    if (video.paused) video.play().catch(() => (broken = true));
    else video.pause();
  }
  function playCut() {
    if (!video || broken) return;
    inLoop = true;
    seek(start);
    video.play().catch(() => (broken = true));
  }
  function ontime() {
    if (!video) return;
    if (!drag) cur = video.currentTime;
    if (inLoop && cur >= end) {
      video.pause();
      inLoop = false;
    }
  }

  // Champs texte : édition précise
  let fromTxt = $state(fmt(start));
  let toTxt = $state(fmt(end));
  $effect(() => {
    fromTxt = fmt(start);
  });
  $effect(() => {
    toTxt = fmt(end);
  });
  function commit(which) {
    const v = parse(which === "start" ? fromTxt : toTxt);
    if (!Number.isFinite(v)) return which === "start" ? (fromTxt = fmt(start)) : (toTxt = fmt(end));
    if (which === "start") start = Math.min(Math.max(0, v), end - 1);
    else end = Math.max(Math.min(D, v), start + 1);
    seek(which === "start" ? start : end);
  }

  function key(e) {
    if (e.target.tagName === "INPUT") return;
    if (e.key === "Escape") onclose();
    else if (e.key === " ") (e.preventDefault(), toggle());
    else if (e.key === "ArrowLeft") seek(cur - 5);
    else if (e.key === "ArrowRight") seek(cur + 5);
    else if (e.key === "i" || e.key === "[") start = Math.min(cur, end - 1);
    else if (e.key === "o" || e.key === "]") end = Math.max(cur, start + 1);
  }
  const len = $derived(end - start);
  const lenTxt = $derived(len >= 60 ? `${Math.floor(len / 60)} min ${two(Math.round(len % 60))}` : `${Math.round(len)} s`);
</script>

<svelte:window onkeydown={key} />

<div class="backdrop" transition:fade={{ duration: 120 }} onclick={onclose} role="presentation"></div>
<div class="dialog" transition:scale={{ start: 0.97, duration: 140 }} role="dialog" aria-label="Choisir l'extrait">
  <header>
    <Icon name="scissors" size={16} />
    <b title={info.title}>{info.title || "Extrait"}</b>
    <button class="x" title="Fermer" onclick={onclose}><Icon name="close" size={14} /></button>
  </header>

  <div class="screen">
    {#if broken}
      {#if info.thumb}<img src={info.thumb} alt="" referrerpolicy="no-referrer" />{/if}
      <span class="nopv">Aperçu indisponible pour ce site — place les curseurs avec les temps.</span>
    {:else}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video
        bind:this={video}
        src={info.preview}
        poster={info.thumb}
        preload="auto"
        bind:paused
        ontimeupdate={ontime}
        onerror={() => (broken = true)}
        onloadedmetadata={() => seek(cur)}
        onclick={toggle}
      ></video>
      {#if paused}<button class="bigplay" onclick={toggle} aria-label="Lecture"><Icon name="play" size={26} /></button>{/if}
    {/if}
  </div>

  <div class="ctl">
    <button class="ib" onclick={toggle} disabled={broken} title="Lecture / pause (espace)"><Icon name={paused ? "play" : "pause"} size={15} /></button>
    <span class="now">{fmt(cur, true)} <i>/ {fmt(D)}</i></span>
    <span class="sp"></span>
    <button class="mini" onclick={() => (start = Math.min(cur, end - 1))} title="Raccourci : I"><b>[</b> Début ici</button>
    <button class="mini" onclick={() => (end = Math.max(cur, start + 1))} title="Raccourci : O">Fin ici <b>]</b></button>
    <button class="mini hot" onclick={playCut} disabled={broken}><Icon name="play" size={11} />Lire l'extrait</button>
  </div>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="track" bind:this={track} onpointerdown={(e) => down(e, "seek")} onpointermove={move} onpointerup={up} onpointercancel={up}>
    {#if info.thumb}<div class="film" style="background-image:url({info.thumb})"></div>{/if}
    <div class="dim" style="left:0;width:{pct(start)}"></div>
    <div class="dim" style="left:{pct(end)};right:0"></div>
    <div class="sel" style="left:{pct(start)};width:{pct(end - start)}"></div>
    <div class="head" style="left:{pct(cur)}"></div>
    <div class="h hs" style="left:{pct(start)}" onpointerdown={(e) => down(e, "start")} role="slider" aria-label="Début" aria-valuenow={start} tabindex="-1"><span></span></div>
    <div class="h he" style="left:{pct(end)}" onpointerdown={(e) => down(e, "end")} role="slider" aria-label="Fin" aria-valuenow={end} tabindex="-1"><span></span></div>
  </div>

  <footer>
    <label>De <input bind:value={fromTxt} onchange={() => commit("start")} spellcheck="false" /></label>
    <label>à <input bind:value={toTxt} onchange={() => commit("end")} spellcheck="false" /></label>
    <span class="len">Extrait : <b>{lenTxt}</b></span>
    <span class="sp"></span>
    <button class="btn" onclick={onclose}>Annuler</button>
    <button class="btn primary" onclick={() => ondone({ start: Math.floor(start), end: Math.ceil(end) })}><Icon name="check" size={14} />Valider</button>
  </footer>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 150; background: var(--scrim); }
  .dialog {
    position: fixed; z-index: 151; left: 50%; top: 50%; translate: -50% -50%;
    width: min(760px, 94vw); max-height: 94vh; overflow: auto;
    display: flex; flex-direction: column; gap: 12px; padding: 16px 18px 18px;
    border-radius: 16px; background: var(--bg-2); border: 1px solid var(--line-strong); box-shadow: var(--shadow-lg);
  }
  header { display: flex; align-items: center; gap: 8px; color: var(--accent); }
  header b { flex: 1; min-width: 0; color: var(--text); font-size: 15px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .x { display: grid; place-items: center; width: 28px; height: 28px; border: 0; border-radius: 8px; background: none; color: var(--muted); cursor: pointer; }
  .x:hover { background: var(--chip); color: var(--text); }
  .screen { position: relative; aspect-ratio: 16 / 9; border-radius: 12px; overflow: hidden; background: #000; display: grid; place-items: center; }
  .screen video, .screen img { width: 100%; height: 100%; object-fit: contain; }
  .screen img { opacity: 0.45; }
  .nopv { position: absolute; padding: 8px 14px; border-radius: 10px; background: rgb(0 0 0 / 0.7); color: white; font-size: 13px; }
  .bigplay { position: absolute; width: 64px; height: 64px; border-radius: 50%; border: 0; display: grid; place-items: center; padding-left: 4px; background: rgb(0 0 0 / 0.55); color: white; cursor: pointer; }
  .bigplay:hover { background: var(--accent); }
  .ctl { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .ib { display: grid; place-items: center; width: 34px; height: 34px; border-radius: 9px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); cursor: pointer; }
  .now { font-size: 13px; font-weight: 700; font-variant-numeric: tabular-nums; }
  .now i { font-style: normal; color: var(--muted); font-weight: 500; }
  .sp { flex: 1; }
  .mini { display: inline-flex; align-items: center; gap: 5px; height: 30px; padding: 0 10px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text-2); font: inherit; font-size: 12.5px; font-weight: 600; cursor: pointer; }
  .mini:hover { color: var(--text); border-color: var(--accent); }
  .mini b { color: var(--accent); }
  .mini.hot { background: var(--accent); border-color: var(--accent); color: white; }
  .mini:disabled, .ib:disabled { opacity: 0.4; cursor: default; }

  .track { position: relative; height: 54px; border-radius: 10px; background: var(--chip); cursor: pointer; touch-action: none; user-select: none; margin: 0 8px; }
  .film { position: absolute; inset: 0; border-radius: 10px; background-size: auto 100%; background-repeat: repeat-x; opacity: 0.55; }
  .dim { position: absolute; top: 0; bottom: 0; background: rgb(0 0 0 / 0.55); }
  .dim:first-of-type { border-radius: 10px 0 0 10px; }
  .sel { position: absolute; top: 0; bottom: 0; border-top: 3px solid var(--accent); border-bottom: 3px solid var(--accent); box-sizing: border-box; }
  .head { position: absolute; top: -4px; bottom: -4px; width: 2px; margin-left: -1px; background: white; box-shadow: 0 0 0 1px rgb(0 0 0 / 0.4); pointer-events: none; }
  .h { position: absolute; top: -2px; bottom: -2px; width: 16px; margin-left: -8px; cursor: ew-resize; display: grid; place-items: center; }
  .h span { width: 12px; height: 100%; border-radius: 4px; background: var(--accent); box-shadow: 0 2px 6px rgb(0 0 0 / 0.35); }
  .h span::after { content: ""; display: block; width: 2px; height: 16px; margin: 17px auto 0; border-radius: 2px; background: rgb(255 255 255 / 0.8); }
  footer { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  footer label { display: inline-flex; align-items: center; gap: 6px; font-size: 12.5px; color: var(--muted); }
  footer input { width: 76px; height: 32px; padding: 0 8px; border-radius: 8px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: inherit; font-size: 13px; text-align: center; font-variant-numeric: tabular-nums; }
  footer input:focus { outline: none; border-color: var(--accent); box-shadow: 0 0 0 3px var(--ring); }
  .len { font-size: 12.5px; color: var(--muted); }
  .len b { color: var(--text); }
</style>
