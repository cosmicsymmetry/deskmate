import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import "./styles.css";

function App() {
  return (
    <main>
      <p className="eyebrow">Deskmate</p>
      <h1>Settings are starting up.</h1>
      <p>
        The background device session is running. Widget setup arrives in the next
        milestone task.
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
