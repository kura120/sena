import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { WidgetBar } from "./WidgetBar";
import "../../styles/theme.css";
import "../../styles/index.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <WidgetBar />
  </StrictMode>
);
