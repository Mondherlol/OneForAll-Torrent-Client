<script>
  // Avant de télécharger une vidéo : qualité (avec taille estimée), MP3, vidéo entière ou un passage.
  import { fade, scale } from "svelte/transition";
  import Cutter from "./Cutter.svelte";
  import Icon from "./Icon.svelte";
  import { fmtSize } from "./model.js";

  /** urls : liens de vidéos ; info : media_info du lien (si un seul), infoErr ; onconfirm({ format, section }) ; onclose() */
  let { urls, info = null, infoErr = "", onconfirm, onclose } = $props();

  const single = $derived(urls.length === 1);
  const resName = (h) => (h >= 2000 ? "4K" : h >= 1400 ? "1440p" : `${h}p`);
  const RES_HINT = { 2160: "Ultra HD", 1440: "Quad HD", 1080: "Full HD", 720: "HD", 480: "Léger", 360: "Très léger" };

  // Choix proposés : les définitions de la vidéo (les 4 plus grandes), sinon les classiques
  const choices = $derived.by(() => {
    if (info?.heights?.length) {
      const hs = info.heights.filter((h) => h >= 360).slice(0, 4);
      return hs.map((h, i) => ({ id: i === 0 ? "max" : String(h), label: resName(h), hint: RES_HINT[h] ?? "", size: info.sizes?.find((s) => s[0] === h)?.[1] ?? 0, best: i === 0 }));
    }
    return [
      { id: "max", label: "Meilleure", hint: "Qualité maximale dispo", best: true },
      { id: "1080", label: "1080p", hint: "Full HD" },
      { id: "720", label: "720p", hint: "HD, plus léger" },
    ];
  });
  let format = $state(localStorage.getItem("oneforall.dlformat") || "max");
  $effect(() => {
    if (format !== "mp3" && !choices.some((c) => c.id === format)) format = "max";
  });

  // Passage
  let part = $state(false);
  let start = $state(0);
  let end = $state(null);
  let cutter = $state(false);
  const two = (n) => String(n).padStart(2, "0");
  const fmt = (t) => {
    const h = Math.floor(t / 3600), m = Math.floor((t % 3600) / 60), s = Math.floor(t % 60);
    return h ? `${h}:${two(m)}:${two(s)}` : `${m}:${two(s)}`;
  };
  const dur = $derived(info?.duration ?? 0);
  const ratio = $derived(part && end != null && dur ? (end - start) / dur : 1);
  const est = (size) => (size > 0 ? `≈ ${fmtSize(size * ratio)}` : "");

  function go() {
    localStorage.setItem("oneforall.dlformat", format);
    onconfirm({ format, section: part && end != null ? `${Math.floor(start)}-${Math.ceil(end)}` : null });
  }
  function key(e) {
    if (cutter) return;
    if (e.key === "Escape") onclose();
    else if (e.key === "Enter") go();
  }
</script>

<svelte:window onkeydown={key} />

<div class="backdrop" transition:fade={{ duration: 120 }} onclick={onclose} role="presentation"></div>
<div class="dialog" transition:scale={{ start: 0.97, duration: 140 }} role="dialog" aria-label="Options de téléchargement">
  <div class="head">
    {#if single && info}
      {#if info.thumb}<img src={info.thumb} alt="" referrerpolicy="no-referrer" />{/if}
      <div class="ht">
        <b title={info.title}>{info.title}</b>
        <span>{dur ? fmt(dur) : ""}{info.heights?.[0] ? ` · jusqu'en ${resName(info.heights[0])}` : ""}</span>
      </div>
    {:else if single && infoErr}
      <div class="ht"><b>Vidéo</b><span class="err"><Icon name="alert" size={12} />{infoErr}</span></div>
    {:else if single}
      <div class="ph"><span class="spin"></span></div>
      <div class="ht"><b>Lecture de la vidéo…</b><span>titre, durée, qualités disponibles</span></div>
    {:else}
      <span class="many"><Icon name="layers" size={20} /></span>
      <div class="ht"><b>{urls.length} vidéos</b><span>Les mêmes options pour toutes</span></div>
    {/if}
    <button class="x" title="Fermer (Échap)" onclick={onclose}><Icon name="close" size={14} /></button>
  </div>

  <h4>Qualité</h4>
  <div class="grid">
    {#each choices as c}
      <button class="opt" class:on={format === c.id} onclick={() => (format = c.id)}>
        <Icon name="film" size={16} />
        <span class="ot"><b>{c.label}{#if c.best}<em>max</em>{/if}</b><i>{c.hint}</i></span>
        <span class="os">{est(c.size)}</span>
      </button>
    {/each}
    <button class="opt" class:on={format === "mp3"} onclick={() => (format = "mp3")}>
      <Icon name="audio" size={16} />
      <span class="ot"><b>MP3</b><i>Juste le son, avec pochette</i></span>
      <span class="os">{est(info?.mp3 ?? 0)}</span>
    </button>
  </div>

  {#if single}
    <h4>Durée</h4>
    <div class="parts">
      <button class="opt" class:on={!part} onclick={() => (part = false)}>
        <Icon name="check" size={15} />
        <span class="ot"><b>Vidéo entière</b>{#if dur}<i>{fmt(dur)}</i>{/if}</span>
      </button>
      <button class="opt" class:on={part} disabled={!info} onclick={() => (end == null ? (cutter = true) : (part = true))}>
        <Icon name="scissors" size={15} />
        <span class="ot">
          <b>Un passage</b>
          <i>{part && end != null ? `${fmt(start)} → ${fmt(end)}` : "Choisir sur la timeline"}</i>
        </span>
        {#if part && end != null}<span class="edit" role="button" tabindex="0" onclick={(e) => (e.stopPropagation(), (cutter = true))} onkeydown={() => {}}>Modifier</span>{/if}
      </button>
    </div>
  {/if}

  <div class="btns">
    <button class="btn" onclick={onclose}>Annuler</button>
    <button class="btn primary" onclick={go}><Icon name="download" size={14} />Télécharger</button>
  </div>
</div>

{#if cutter && info}
  <Cutter
    {info}
    start={start}
    end={end}
    onclose={() => (cutter = false)}
    ondone={(r) => {
      start = r.start;
      end = r.end;
      part = true;
      cutter = false;
    }}
  />
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 145; background: var(--scrim); }
  .dialog {
    position: fixed; z-index: 146; left: 50%; top: 46%; translate: -50% -50%;
    width: min(520px, 94vw); max-height: 92vh; overflow: auto; padding: 16px 18px 18px;
    border-radius: 18px; background: var(--bg-2); border: 1px solid var(--line-strong); box-shadow: var(--shadow-lg);
  }
  .head { display: flex; align-items: center; gap: 12px; padding-bottom: 14px; border-bottom: 1px solid var(--line); }
  .head img, .ph, .many { width: 120px; height: 68px; flex: none; border-radius: 10px; object-fit: cover; background: var(--chip); display: grid; place-items: center; color: var(--muted); }
  .many { width: 48px; height: 48px; }
  .ht { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .ht b { font-size: 15px; line-height: 1.3; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .ht span { font-size: 12.5px; color: var(--muted); font-variant-numeric: tabular-nums; }
  .ht .err { display: inline-flex; align-items: center; gap: 5px; color: var(--err); }
  .x { align-self: flex-start; display: grid; place-items: center; width: 28px; height: 28px; border: 0; border-radius: 8px; background: none; color: var(--muted); cursor: pointer; }
  .x:hover { background: var(--chip); color: var(--text); }
  h4 { margin: 16px 0 8px; font-size: 11.5px; letter-spacing: 0.06em; text-transform: uppercase; color: var(--muted); }
  .grid, .parts { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .opt { position: relative; display: flex; align-items: center; gap: 10px; padding: 10px 12px; border-radius: 12px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text-2); font: inherit; text-align: left; cursor: pointer; }
  .opt:hover:not(:disabled) { border-color: var(--accent); color: var(--text); }
  .opt.on { border-color: var(--accent); box-shadow: 0 0 0 3px var(--ring); color: var(--accent); }
  .opt:disabled { opacity: 0.5; cursor: default; }
  .ot { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .ot b { display: inline-flex; align-items: center; gap: 6px; font-size: 13.5px; color: var(--text); }
  .ot b em { font-style: normal; font-size: 9.5px; font-weight: 800; letter-spacing: 0.05em; text-transform: uppercase; padding: 1px 5px; border-radius: 4px; background: var(--accent-soft); color: var(--accent); }
  .ot i { font-style: normal; font-size: 11.5px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-variant-numeric: tabular-nums; }
  .os { font-size: 11.5px; font-weight: 650; color: var(--text-2); font-variant-numeric: tabular-nums; white-space: nowrap; }
  .edit { font-size: 11.5px; font-weight: 700; color: var(--accent); padding: 3px 7px; border-radius: 6px; background: var(--accent-soft); }
  .btns { display: flex; justify-content: flex-end; gap: 8px; margin-top: 18px; }
  .spin { width: 18px; height: 18px; border-radius: 50%; border: 2px solid var(--line-strong); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (max-width: 480px) { .grid, .parts { grid-template-columns: 1fr; } }
</style>
