import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { EventBus } from "./EventBus";
import "../../styles/theme.css";
import "../../styles/index.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <EventBus />
  </StrictMode>
);
