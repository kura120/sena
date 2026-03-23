import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "../../styles/index.css";
import { Resources } from "./Resources";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Resources />
  </StrictMode>
);
