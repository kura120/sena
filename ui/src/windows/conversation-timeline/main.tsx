import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "../../styles/index.css";
import { ConversationTimeline } from "./ConversationTimeline";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ConversationTimeline />
  </StrictMode>
);
