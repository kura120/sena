import React from "react";
import ReactDOM from "react-dom/client";
import { ToastWindow } from "./ToastWindow";
import "../../styles/index.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <div
      style={{
        width: "100%",
        minHeight: "1px",
        overflow: "visible",
        background: "transparent",
        pointerEvents: "none",
      }}
    >
      <ToastWindow />
    </div>
  </React.StrictMode>,
);
