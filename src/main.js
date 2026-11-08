import { mount } from "svelte";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./app.css";
import App from "./App.svelte";
import Overlay from "./lib/Overlay.svelte";

// Fenêtre « Épisode suivant » (ouverte par-dessus VLC) : même page, autre fenêtre
let overlay = false;
try {
  overlay = getCurrentWindow().label === "overlay";
} catch {}

export default mount(overlay ? Overlay : App, { target: document.getElementById("app") });
