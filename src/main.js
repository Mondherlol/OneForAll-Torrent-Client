import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";
import Overlay from "./lib/Overlay.svelte";

// Fenêtre « Épisode suivant » (ouverte par-dessus VLC) : index.html#overlay=<données>
const overlay = location.hash.startsWith("#overlay=");

export default mount(overlay ? Overlay : App, { target: document.getElementById("app") });
