import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "../../styles/index.css";
import { ThoughtStream } from "./ThoughtStream";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ThoughtStream />
  </StrictMode>
);
