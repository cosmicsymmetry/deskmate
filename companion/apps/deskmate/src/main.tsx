import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { useAppState } from "./lib/useAppState";
import "./styles.css";

function App() {
  const { snapshot, loading, error } = useAppState();
  const connection = snapshot?.device.connection.kind;
  const status = error
    ? `Background service error: ${error.message}`
    : loading
      ? "Connecting to the background service…"
      : `Background service ready · Device ${connection ?? "unknown"}`;

  return (
    <main>
      <p className="eyebrow">Deskmate</p>
      <h1>Settings foundation is ready.</h1>
      <p>
        {status}. Widget setup arrives in the next milestone task; the device session
        remains owned by the app when this window is hidden or reloaded.
      </p>
    </main>
  );
}

const root = document.getElementById("root");

if (!root) {
  throw new Error("missing React root");
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
