import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { checkCli, loadDevices, pollStatus, restoreAfterElevation } from "./store/actions";
import { startBridge } from "./store/bridge";
import "./styles.css";

async function boot() {
  // In a plain browser (`npm run dev` without the Tauri window) a mock stands in for the Rust side.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) await import("./dev/devmock");
  await startBridge();
  await restoreAfterElevation();

  const root = document.getElementById("root");
  if (!root) throw new Error("index.html is missing #root");
  createRoot(root).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );

  void checkCli();
  void loadDevices();
  void pollStatus();
}

void boot();
