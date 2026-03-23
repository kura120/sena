import React from "react";
import ReactDOM from "react-dom/client";
import { NotificationHistory } from "../notification-history/NotificationHistory"
import "../../styles/index.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <NotificationHistory />
  </React.StrictMode>,
);
