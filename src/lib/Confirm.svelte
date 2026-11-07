<script>
  import { fade, scale } from "svelte/transition";
  import { ui } from "./ui.svelte.js";

  function done(v) {
    ui.confirm?.resolve(v);
    ui.confirm = null;
  }
</script>

<svelte:window onkeydown={(e) => ui.confirm && e.key === "Escape" && done(false)} />

{#if ui.confirm}
  <div class="backdrop" transition:fade={{ duration: 120 }} onclick={() => done(false)} role="presentation"></div>
  <div class="dialog" transition:scale={{ start: 0.96, duration: 140 }} role="alertdialog">
    <h3>{ui.confirm.title}</h3>
    {#if ui.confirm.text}<p>{ui.confirm.text}</p>{/if}
    <div class="btns">
      <button class="btn" onclick={() => done(false)}>Annuler</button>
      <!-- svelte-ignore a11y_autofocus -->
      <button class="btn primary" class:danger-fill={ui.confirm.danger} autofocus onclick={() => done(true)}>{ui.confirm.ok}</button>
    </div>
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 150; background: var(--scrim); }
  .dialog {
    position: fixed;
    z-index: 151;
    left: 50%;
    top: 40%;
    translate: -50% -50%;
    width: min(420px, 90vw);
    padding: 22px;
    border-radius: 16px;
    background: var(--bg-2);
    border: 1px solid var(--line-strong);
    box-shadow: var(--shadow-lg);
  }
  h3 { margin: 0 0 8px; font-size: 17px; }
  p { margin: 0; color: var(--text-2); line-height: 1.5; white-space: pre-line; }
  .btns { display: flex; justify-content: flex-end; gap: 8px; margin-top: 20px; }
</style>
