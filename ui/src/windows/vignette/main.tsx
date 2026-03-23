import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import Vignette from "./Vignette";
import "../../styles/index.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Vignette />
  </StrictMode>
);
