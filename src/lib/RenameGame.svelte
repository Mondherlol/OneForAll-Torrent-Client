<script>
  // « Vrai nom du jeu » : sert à trouver la jaquette et les infos ; affiché sur la carte si on veut.
  import { fade, scale } from "svelte/transition";
  import Icon from "./Icon.svelte";
  import { artChoices, setArtwork } from "./api.js";
  import { arts, flash, gameNames, setGameName, ui } from "./ui.svelte.js";

  const job = ui.rename; // { key, title }
  const prev = gameNames[job.key];
  let name = $state(prev?.name ?? job.title);
  let show = $state(prev?.show ?? true);
  let picked = $state(null); // suggestion IGDB choisie : sa jaquette est appliquée directement

  // Suggestions IGDB pendant la saisie
  let sugg = $state([]);
  let loading = $state(false);
  let run = 0;
  $effect(() => {
    const q = name.trim();
    if (q.length < 2) return void (sugg = []);
    const id = ++run;
    const t = setTimeout(async () => {
      loading = true;
      try {
        const l = await artChoices("game", q, null);
        if (id !== run) return;
        const seen = new Set();
        sugg = l.filter((c) => c.source === "IGDB" && !seen.has(c.title) && seen.add(c.title)).slice(0, 6);
      } catch {
        if (id === run) sugg = [];
      } finally {
        if (id === run) loading = false;
      }
    }, 350);
    return () => clearTimeout(t);
  });

  function choose(c) {
    name = c.title;
    picked = c;
  }
  const close = () => (ui.rename = null);

  async function save() {
    const n = name.trim();
    if (!n) return;
    setGameName(job.key, n === job.title && !picked ? "" : n, show);
    if (picked && picked.title === n) {
      const key = `game:named:${n.toLowerCase()}`;
      try {
        arts[key] = await setArtwork(key, { url: picked.url, fallback: picked.fallback ?? null });
      } catch (e) {
        flash(String(e));
      }
    }
    close();
  }
  function reset() {
    setGameName(job.key, "", true);
    close();
  }
  function key(e) {
    if (e.key === "Escape") close();
    else if (e.key === "Enter") save();
  }
</script>

<svelte:window onkeydown={key} />

<div class="backdrop" transition:fade={{ duration: 120 }} onclick={close} role="presentation"></div>
<div class="dialog" transition:scale={{ start: 0.97, duration: 140 }} role="dialog" aria-label="Vrai nom du jeu">
  <header>
    <span class="ic"><Icon name="game" size={18} /></span>
    <div>
      <h3>Vrai nom du jeu</h3>
      <p>Sert à trouver la jaquette et les infos du jeu</p>
    </div>
    <button class="x" onclick={close} title="Fermer"><Icon name="close" size={16} /></button>
  </header>

  <!-- svelte-ignore a11y_autofocus -->
  <input bind:value={name} autofocus placeholder="Ex. Lost in Blue" oninput={() => (picked = null)} />
  <small class="orig">Nom du fichier : {job.title}</small>

  <div class="sugg">
    {#if loading && !sugg.length}
      {#each Array(3) as _}<span class="s sk"></span>{/each}
    {:else}
      {#each sugg as c (c.title)}
        <button class="s" class:on={picked === c} onclick={() => choose(c)} title="Utiliser ce nom et cette jaquette">
          <img src={c.fallback || c.url} alt="" />
          <span><b>{c.title}</b>{#if c.sub}<small>{c.sub}</small>{/if}</span>
          {#if picked === c}<Icon name="check" size={16} />{/if}
        </button>
      {/each}
    {/if}
  </div>

  <label class="tog">
    <input type="checkbox" bind:checked={show} />
    <span class="sw"></span>
    <span>Afficher ce nom sur la carte <small>sinon la carte garde le nom du fichier</small></span>
  </label>

  <footer>
    {#if prev}<button class="btn ghost" onclick={reset}>Revenir au nom d'origine</button>{/if}
    <button class="btn" onclick={close}>Annuler</button>
    <button class="btn primary" disabled={!name.trim()} onclick={save}><Icon name="check" size={15} />Enregistrer</button>
  </footer>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 70; background: var(--scrim); backdrop-filter: blur(3px); }
  .dialog { position: fixed; z-index: 71; left: 50%; top: 12vh; translate: -50% 0; width: min(520px, 94vw); padding: 20px; border-radius: 18px; background: var(--bg-2); border: 1px solid var(--line-strong); box-shadow: var(--shadow-lg); display: flex; flex-direction: column; gap: 12px; }
  header { display: flex; align-items: center; gap: 12px; }
  header h3 { margin: 0; font-size: 17px; }
  header p { margin: 2px 0 0; font-size: 12.5px; color: var(--muted); }
  .ic { display: grid; place-items: center; width: 40px; height: 40px; border-radius: 12px; background: var(--accent-soft); color: var(--accent); flex: none; }
  .x { margin-left: auto; align-self: flex-start; display: grid; place-items: center; width: 32px; height: 32px; border-radius: 9px; border: 0; background: none; color: var(--muted); cursor: pointer; }
  .x:hover { background: var(--chip); color: var(--text); }
  input:not([type]) { height: 46px; padding: 0 14px; border-radius: 12px; border: 1.5px solid var(--line-strong); background: var(--surface); color: var(--text); font: inherit; font-size: 15px; font-weight: 600; outline: none; }
  input:not([type]):focus { border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-soft); }
  .orig { margin-top: -6px; font-size: 11.5px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sugg { display: flex; flex-direction: column; gap: 6px; max-height: 300px; overflow-y: auto; }
  .s { display: flex; align-items: center; gap: 12px; padding: 6px 12px 6px 6px; border-radius: 12px; border: 1.5px solid var(--line); background: var(--surface); color: var(--text); font: inherit; text-align: left; cursor: pointer; }
  .s:hover { border-color: var(--accent); }
  .s.on { border-color: var(--accent); background: var(--accent-soft); color: var(--accent); }
  .s img { width: 38px; height: 50px; object-fit: cover; border-radius: 6px; background: var(--chip); flex: none; }
  .s span { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .s b { font-size: 13.5px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; color: var(--text); }
  .s small { font-size: 11.5px; color: var(--muted); }
  .s.sk { height: 62px; border: 0; background: linear-gradient(100deg, var(--chip) 30%, var(--line) 50%, var(--chip) 70%); background-size: 300% 100%; animation: sh 1.2s linear infinite; }
  @keyframes sh { from { background-position: 100% 0; } to { background-position: 0 0; } }
  .tog { display: flex; align-items: center; gap: 10px; font-size: 13.5px; cursor: pointer; }
  .tog input { display: none; }
  .tog small { display: block; font-size: 11.5px; color: var(--muted); }
  .sw { position: relative; width: 38px; height: 22px; border-radius: 22px; background: var(--line-strong); flex: none; transition: background 0.15s; }
  .sw::after { content: ""; position: absolute; left: 3px; top: 3px; width: 16px; height: 16px; border-radius: 50%; background: white; transition: transform 0.15s; box-shadow: 0 1px 3px rgb(0 0 0 / 0.3); }
  .tog input:checked + .sw { background: var(--accent); }
  .tog input:checked + .sw::after { transform: translateX(16px); }
  footer { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
  footer .ghost { margin-right: auto; border-color: transparent; background: none; color: var(--muted); }
</style>
