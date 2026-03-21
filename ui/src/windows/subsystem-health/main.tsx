import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { SubsystemHealth } from "./SubsystemHealth";
import "../../styles/theme.css";
import "../../styles/index.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <SubsystemHealth />
  </StrictMode>
);
