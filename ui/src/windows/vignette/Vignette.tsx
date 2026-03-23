import { useState, useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import "../../styles/theme.css";
import "../../styles/index.css";

interface OverlayAnimatePayload {
  action: "show" | "hide";
  delay_ms: number;
}

function Vignette() {
  const [className, setClassName] = useState("vignette-bg");

  useEffect(() => {
    let cancelled = false;

    const unlistenPromise = listen<OverlayAnimatePayload>("overlay-animate", (event) => {
      if (cancelled) return;
      const { action } = event.payload;

      if (action === "show") {
        setClassName("vignette-bg vignette-active");
      } else if (action === "hide") {
        setClassName("vignette-bg vignette-hiding");
      }
    });

    return () => {
      cancelled = true;
      unlistenPromise.then((fn) => fn());
    };
  }, []);

  return <div className={className} />;
}

export default Vignette;
