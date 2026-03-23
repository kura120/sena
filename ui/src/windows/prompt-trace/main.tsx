import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "../../styles/index.css";
import { PromptTrace } from "./PromptTrace";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <PromptTrace />
  </StrictMode>
);
