<script>
  // Fenêtre « Épisode suivant », affichée par-dessus VLC quand le générique de fin commence.
  // Ses données viennent de la commande overlay_data :
  //   mode « countdown » : compte à rebours puis épisode suivant
  //   mode « scene »     : une scène suit le générique (after = sa durée) → pas de compte à rebours
  //   mode « end »       : fin du fichier après la scène → compte à rebours
  import { invoke } from "@tauri-apps/api/core";
  import Icon from "./Icon.svelte";

  let data = $state(null);
  let total = $state(15);
  let left = $state(15);
  let done = false;
  let artOk = $state(false);
  const scene = $derived(data?.mode === "scene");

  function act(action) {
    if (done) return;
    done = true;
    invoke("overlay_action", { action }).catch(() => {});
  }

  $effect(() => {
    // Fenêtre transparente : seule la carte se voit
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";
    let id;
    invoke("overlay_data")
      .then((d) => {
        data = d ?? {};
        artOk = !!data.art;
        if (data.mode === "scene") return; // pas de compte à rebours : on attend le choix
        total = Math.max(3, data.seconds ?? 15);
        left = total;
        const started = Date.now();
        id = setInterval(() => {
          left = Math.max(0, total - (Date.now() - started) / 1000);
          if (left <= 0) {
            clearInterval(id);
            act("next");
          }
        }, 100);
      })
      .catch(() => (data = {}));
    return () => clearInterval(id);
  });

  const fmt = (s) => (s >= 60 ? `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, "0")}` : `${s} s`);
</script>

{#if data}
  <div class="card" class:scene role="dialog" aria-label="Épisode suivant">
    {#if artOk}
      <img class="poster" src={data.art} alt="" onerror={() => (artOk = false)} />
    {:else}
      <span class="poster ph"><Icon name="series" size={26} /></span>
    {/if}
    <div class="body">
      {#if scene}
        <span class="kicker warn"><Icon name="alert" size={11} />Scène après le générique · {fmt(data.after ?? 0)}</span>
        <b class="title">Ne pars pas tout de suite</b>
        <span class="show">Ensuite : {data.label || data.name}{data.title ? ` — ${data.title}` : ""}</span>
      {:else}
        <span class="kicker">Épisode suivant</span>
        <b class="title">{data.label || data.name}</b>
        <span class="show" title={data.name}>{data.title}</span>
      {/if}
      <div class="actions">
        {#if scene}
          <button class="primary" onclick={() => act("skip")}><Icon name="chevRight" size={14} />Passer le générique</button>
          <button class="ghost" onclick={() => act("next")}>Épisode suivant</button>
        {:else}
          <button class="go" onclick={() => act("next")} style="--p:{(1 - left / total) * 100}%">
            <span class="fill"></span>
            <span class="lbl"><Icon name="play" size={13} />Lancer · {Math.ceil(left)}</span>
          </button>
          <button class="ghost" onclick={() => act("cancel")}>Annuler</button>
        {/if}
      </div>
    </div>
    {#if scene}
      <button class="x" title="Fermer (la suite sera proposée à la fin)" onclick={() => act("cancel")}><Icon name="close" size={13} /></button>
    {/if}
  </div>
{/if}

<style>
  :global(html, body) { margin: 0; overflow: hidden; background: transparent; user-select: none; }
  .card {
    position: fixed;
    inset: 6px;
    display: flex;
    gap: 14px;
    padding: 12px 14px 12px 12px;
    border-radius: 16px;
    background: rgb(14 16 22 / 0.9);
    border: 1px solid rgb(255 255 255 / 0.12);
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.5);
    backdrop-filter: blur(14px);
    color: white;
    font-family: "Segoe UI Variable", "Segoe UI", system-ui, sans-serif;
    animation: rise 0.35s cubic-bezier(0.2, 0.9, 0.3, 1.2);
  }
  .card.scene { border-color: rgb(251 191 36 / 0.45); }
  @keyframes rise { from { opacity: 0; transform: translateY(18px) scale(0.97); } }
  .poster { width: 84px; height: 120px; flex: none; object-fit: cover; border-radius: 9px; box-shadow: 0 6px 16px rgb(0 0 0 / 0.5); }
  .poster.ph { display: grid; place-items: center; background: rgb(255 255 255 / 0.08); color: rgb(255 255 255 / 0.6); }
  .body { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .kicker { display: inline-flex; align-items: center; gap: 5px; font-size: 10.5px; font-weight: 800; letter-spacing: 0.12em; text-transform: uppercase; color: #4cc2ff; }
  .kicker.warn { color: #fbbf24; letter-spacing: 0.06em; }
  .title { margin-top: 3px; font-size: 19px; font-weight: 800; letter-spacing: -0.01em; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .scene .title { font-size: 17px; }
  .show { font-size: 12.5px; color: rgb(255 255 255 / 0.7); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .actions { margin-top: auto; display: flex; gap: 8px; }
  button { height: 34px; border-radius: 9px; border: 0; font: inherit; font-size: 13px; font-weight: 700; cursor: pointer; }
  .go { position: relative; flex: 1; overflow: hidden; background: rgb(255 255 255 / 0.18); color: #111; }
  .go .fill { position: absolute; inset: 0; width: var(--p); background: white; transition: width 0.1s linear; }
  .go .lbl { position: relative; display: inline-flex; align-items: center; justify-content: center; gap: 6px; width: 100%; mix-blend-mode: difference; color: white; font-variant-numeric: tabular-nums; }
  .go:hover .fill { width: 100%; }
  .primary { flex: 1; display: inline-flex; align-items: center; justify-content: center; gap: 4px; background: #fbbf24; color: #1a1205; }
  .primary:hover { background: #fcd34d; }
  .ghost { padding: 0 14px; background: rgb(255 255 255 / 0.08); color: rgb(255 255 255 / 0.85); border: 1px solid rgb(255 255 255 / 0.14); }
  .ghost:hover { background: rgb(255 255 255 / 0.16); color: white; }
  .x { position: absolute; top: 8px; right: 8px; width: 24px; height: 24px; display: grid; place-items: center; padding: 0; border-radius: 7px; background: rgb(255 255 255 / 0.08); color: rgb(255 255 255 / 0.7); }
  .x:hover { background: rgb(255 255 255 / 0.18); color: white; }
</style>
