import React from "react";
import ReactDOM from "react-dom/client";
import "./styles/index.css";

/** Host window — invisible, keeps the Tauri process alive. */
function Host() {
  return null;
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Host />
  </React.StrictMode>,
);
