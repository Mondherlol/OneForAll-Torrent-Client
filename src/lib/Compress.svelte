<script>
  // Compresser une vidéo téléchargée pour la partager (taille estimée pour chaque niveau)
  import { fade, scale } from "svelte/transition";
  import Icon from "./Icon.svelte";
  import { compressInfo, compressVideo } from "./api.js";
  import { fmtSize } from "./model.js";
  import { flash } from "./ui.svelte.js";

  /** d : téléchargement terminé (vidéo) ; onclose() ; ondone() */
  let { d, onclose, ondone } = $props();

  let info = $state(null);
  let err = $state("");
  let pick = $state(null);
  let custom = $state("");
  const MB = 1048576;

  $effect(() => {
    compressInfo(d.path)
      .then((r) => {
        info = r;
        pick = r.levels.find((l) => l[0] === "Moyen")?.[1] ?? r.levels[0]?.[1] ?? null;
      })
      .catch((e) => (err = String(e)));
  });

  const HINT = { Léger: "presque pareil", Moyen: "bon compromis", Fort: "le plus petit", Discord: "limite Discord gratuit", WhatsApp: "limite WhatsApp", Mail: "pièce jointe Gmail" };
  const customBytes = $derived(Math.round(parseFloat(String(custom).replace(",", ".")) * MB) || 0);
  const target = $derived(custom ? customBytes : pick);
  const tooSmall = $derived(info && target && target < info.probe.size * 0.03);

  async function go() {
    if (!target || target >= info.probe.size) return flash("Choisis une taille plus petite que l'original");
    try {
      await compressVideo(d.path, target);
      flash("Compression lancée — elle apparaît dans la liste");
      ondone();
    } catch (e) {
      flash(String(e));
    }
  }
  const pctOf = (s) => Math.round((s / info.probe.size) * 100);
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} />

<div class="backdrop" transition:fade={{ duration: 120 }} onclick={onclose} role="presentation"></div>
<div class="dialog" transition:scale={{ start: 0.97, duration: 140 }} role="dialog" aria-label="Compresser">
  <h3><Icon name="layers" size={16} />Compresser la vidéo</h3>
  <p class="name" title={d.name}>{d.name}</p>

  {#if err}
    <p class="err">{err}</p>
  {:else if !info}
    <p class="load">Analyse de la vidéo…</p>
  {:else}
    <p class="src">
      Original : <b>{fmtSize(info.probe.size)}</b> · {info.probe.width}×{info.probe.height} · {Math.floor(info.probe.duration / 60)} min {String(Math.round(info.probe.duration % 60)).padStart(2, "0")}
    </p>
    <div class="levels">
      {#each info.levels as [label, size, h]}
        <button class="lv" class:on={!custom && pick === size} onclick={() => ((pick = size), (custom = ""))}>
          <span class="lt"><b>{label}</b><i>{HINT[label] ?? ""}</i></span>
          <span class="res">{h}p</span>
          <span class="sz">≈ {fmtSize(size)}</span>
          <span class="pc">−{100 - pctOf(size)} %</span>
        </button>
      {/each}
      <label class="lv cust" class:on={!!custom}>
        <span class="lt"><b>Taille au choix</b></span>
        <input bind:value={custom} placeholder="ex. 8" inputmode="decimal" /> Mo
      </label>
    </div>
    {#if tooSmall}<p class="warn">Très petit pour cette durée : l'image sera très dégradée.</p>{/if}
    <p class="hint">Le fichier compressé (MP4) est créé à côté de l'original, qui reste intact.</p>
  {/if}

  <div class="btns">
    <button class="btn" onclick={onclose}>Annuler</button>
    <button class="btn primary" disabled={!info || !target} onclick={go}><Icon name="layers" size={14} />Compresser{target ? ` · ≈ ${fmtSize(target)}` : ""}</button>
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 150; background: var(--scrim); }
  .dialog {
    position: fixed; z-index: 151; left: 50%; top: 45%; translate: -50% -50%;
    width: min(480px, 92vw); padding: 20px 22px; border-radius: 16px;
    background: var(--bg-2); border: 1px solid var(--line-strong); box-shadow: var(--shadow-lg);
  }
  h3 { display: flex; align-items: center; gap: 8px; margin: 0; font-size: 16px; }
  .name { margin: 4px 0 12px; font-size: 12.5px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .src { margin: 0 0 10px; font-size: 13px; color: var(--text-2); }
  .load, .hint { font-size: 12.5px; color: var(--muted); margin: 10px 0 0; }
  .err, .warn { font-size: 13px; color: var(--err); margin: 8px 0 0; }
  .warn { color: #d97706; }
  .levels { display: flex; flex-direction: column; gap: 6px; }
  .lv { display: flex; align-items: center; gap: 12px; padding: 9px 12px; border-radius: 10px; border: 1px solid var(--line-strong); background: var(--surface); color: var(--text); font: inherit; font-size: 13px; text-align: left; cursor: pointer; }
  .lv:hover { border-color: var(--accent); }
  .lv.on { border-color: var(--accent); box-shadow: 0 0 0 3px var(--ring); }
  .lt { flex: 1; display: flex; flex-direction: column; min-width: 0; }
  .lt i { font-style: normal; font-size: 11.5px; color: var(--muted); }
  .res { font-size: 11px; font-weight: 700; padding: 1px 6px; border-radius: 5px; background: var(--chip); color: var(--text-2); }
  .sz { font-weight: 700; font-variant-numeric: tabular-nums; min-width: 72px; text-align: right; }
  .pc { font-size: 12px; color: var(--ok); font-weight: 650; min-width: 44px; text-align: right; }
  .cust { cursor: default; }
  .cust input { width: 70px; height: 28px; padding: 0 8px; border-radius: 7px; border: 1px solid var(--line-strong); background: var(--bg-2); color: var(--text); font: inherit; text-align: right; }
  .cust input:focus { outline: none; border-color: var(--accent); }
  .btns { display: flex; justify-content: flex-end; gap: 8px; margin-top: 18px; }
</style>
