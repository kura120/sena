import React from "react";
import ReactDOM from "react-dom/client";
import { ToastWindow } from "./ToastWindow";
import "../../styles/index.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <div className="h-screen w-screen overflow-hidden bg-transparent flex items-end justify-center pb-8">
        <ToastWindow />
    </div>
  </React.StrictMode>,
);
