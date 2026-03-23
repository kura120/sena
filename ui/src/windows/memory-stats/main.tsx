import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "../../styles/index.css";
import { MemoryStats } from "./MemoryStats";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <MemoryStats />
  </StrictMode>
);
