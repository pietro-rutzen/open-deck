import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import "./theme.css";
import "./styles.css";
import "./native.css";

window.addEventListener("contextmenu", (event) => {
  const target = event.target;
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) return;
  event.preventDefault();
}, true);

const rootEl = document.getElementById("root");
if (rootEl) {
  ReactDOM.createRoot(rootEl).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}
