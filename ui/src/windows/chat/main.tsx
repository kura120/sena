import React from "react";
import ReactDOM from "react-dom/client";
import { Chat } from "./Chat";
import "../../styles/index.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <div className="h-screen w-screen overflow-hidden bg-transparent p-1">
        <Chat />
    </div>
  </React.StrictMode>,
);
